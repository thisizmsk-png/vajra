//! The owner-only Unix socket between the `--mcp` process and the app
//! (design.md 2 "MCP <-> app", 8 "IPC surface").
//!
//! - The socket is 0600 inside a 0700 dir.
//! - Every connection's peer uid is checked with getpeereid before a byte is
//!   read; any other uid gets `{"error":"peer_uid"}` and is closed.
//! - Newline-delimited JSON, one request per line, at most 65536 bytes per
//!   request (newline excluded). A longer line gets `request_too_large`.
//! - Request `{"tool": "...", "args": {...}, "run": "<run_id>"?}`, reply `{"ok": ...}` or
//!   `{"error": "<code>", ...}`. Every request runs as the Lead. A request with `run` is served only while that
//!   run is the current control session (ADR-13: a Lead is bound to the run it was asked about).

use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use super::fsx;
use super::service::Service;

/// Request cap: 64 KiB.
pub const MAX_REQUEST: usize = 64 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// The uid of the process on the other end, via getpeereid (LOCAL_PEERCRED).
#[cfg(not(target_os = "linux"))]
pub fn peer_uid(s: &UnixStream) -> io::Result<u32> {
    let mut uid: libc::uid_t = 0;
    let mut gid: libc::gid_t = 0;
    // SAFETY: valid fd and out-pointers for the duration of the call.
    let rc = unsafe { libc::getpeereid(s.as_raw_fd(), &mut uid, &mut gid) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(uid)
}

/// Linux has no getpeereid; SO_PEERCRED gives the same uid.
#[cfg(target_os = "linux")]
pub fn peer_uid(s: &UnixStream) -> io::Result<u32> {
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: cred and len are valid for the call.
    let rc = unsafe {
        libc::getsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut cred as *mut libc::ucred).cast(), &mut len)
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(cred.uid)
}

/// Bind at `path`: parent dir 0700, a stale socket replaced (anything else
/// in its place is an error), socket file 0600.
pub fn bind(path: &Path) -> io::Result<UnixListener> {
    if let Some(dir) = path.parent() {
        fsx::private_dir(dir)?;
    }
    if let Ok(m) = fs::symlink_metadata(path) {
        if !m.file_type().is_socket() {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, "socket path is not a socket"));
        }
        fs::remove_file(path)?;
    }
    let l = UnixListener::bind(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(l)
}

/// Accept forever, one thread per connection. Only `allowed_uid` is served.
pub fn serve(listener: UnixListener, svc: Arc<Mutex<Service>>, allowed_uid: u32) {
    for conn in listener.incoming() {
        let Ok(conn) = conn else { continue };
        let svc = svc.clone();
        std::thread::spawn(move || {
            let _ = handle(conn, &svc, allowed_uid);
        });
    }
}

fn reply(w: &mut UnixStream, v: &Value) -> io::Result<()> {
    let mut line = v.to_string();
    line.push('\n');
    w.write_all(line.as_bytes())
}

/// Read one line of at most MAX_REQUEST bytes. `Ok(None)` at EOF,
/// `Err(InvalidData)` if the line is longer than the cap.
fn read_request(r: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut buf = Vec::new();
    r.take(MAX_REQUEST as u64 + 1).read_until(b'\n', &mut buf)?;
    if buf.is_empty() {
        return Ok(None);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
    } else if buf.len() > MAX_REQUEST {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "request_too_large"));
    }
    Ok(Some(buf))
}

fn handle(mut conn: UnixStream, svc: &Mutex<Service>, allowed_uid: u32) -> io::Result<()> {
    match peer_uid(&conn) {
        Ok(uid) if uid == allowed_uid => {}
        _ => return reply(&mut conn, &json!({"error": "peer_uid"})),
    }
    conn.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut reader = BufReader::new(conn.try_clone()?);
    loop {
        let req = match read_request(&mut reader) {
            Ok(Some(r)) => r,
            Ok(None) => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                return reply(&mut conn, &json!({"error": "request_too_large"}));
            }
            Err(e) => return Err(e),
        };
        let out = match serde_json::from_slice::<Value>(&req) {
            Ok(v) => match v.get("tool").and_then(Value::as_str) {
                Some(tool) => {
                    let args = v.get("args").cloned().unwrap_or(Value::Null);
                    let mut s = crate::lock(svc);
                    let run = match v.get("run") {
                        None | Some(Value::Null) => None,
                        Some(Value::String(r)) => Some(r.as_str()),
                        Some(_) => {
                            drop(s);
                            reply(&mut conn, &json!({"error": "invalid_args", "detail": "run"}))?;
                            continue;
                        }
                    };
                    match s.call_tool_for_run(run, tool, &args) {
                        Ok(r) => json!({"ok": r}),
                        Err(e) => e.to_json(),
                    }
                }
                None => json!({"error": "bad_request"}),
            },
            Err(_) => json!({"error": "bad_request"}),
        };
        reply(&mut conn, &out)?;
    }
}

/// Client side: one request, one reply line.
pub fn request(path: &Path, tool: &str, args: &Value) -> io::Result<Value> {
    request_for(path, None, tool, args)
}

/// Like `request`, bound to `run` when given.
pub fn request_for(path: &Path, run: Option<&str>, tool: &str, args: &Value) -> io::Result<Value> {
    let mut s = UnixStream::connect(path)?;
    s.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut req = json!({"tool": tool, "args": args});
    if let Some(r) = run {
        req["run"] = json!(r);
    }
    let mut line = req.to_string();
    line.push('\n');
    if line.len() > MAX_REQUEST + 1 {
        return Ok(json!({"error": "request_too_large"}));
    }
    s.write_all(line.as_bytes())?;
    let mut r = BufReader::new(s);
    let mut out = String::new();
    r.read_line(&mut out)?;
    serde_json::from_str(&out).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
