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
//! - A `vajra-run` request (ADR-16 16.5, from a run lead's `--mcp-run` server) carries `"vajra_run"` and `"token"`. It
//!   goes to the `RunRegistry` with the peer's pid and never takes the `Service` lock, so an `await_any` that parks
//!   does not hold up any other call. Only a `plan_tasks` request may use the larger cap, `runreg::MAX_PLAN_REQUEST`.

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
use super::runreg::{self, RunRegistry};
use super::service::Service;

/// Request cap: 64 KiB.
pub const MAX_REQUEST: usize = 64 * 1024;
pub const READ_TIMEOUT: Duration = Duration::from_secs(30);

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

/// The pid of the process on the other end. macOS: the audit token (LOCAL_PEERTOKEN), which names the peer as it was
/// when it connected; its pid is slot 5.
#[cfg(target_os = "macos")]
pub fn peer_pid(s: &UnixStream) -> io::Result<u32> {
    let mut tok = [0u32; 8];
    let mut len = std::mem::size_of_val(&tok) as libc::socklen_t;
    // SAFETY: tok and len are valid for the call; the kernel writes at most len bytes.
    let rc = unsafe { libc::getsockopt(s.as_raw_fd(), libc::SOL_LOCAL, libc::LOCAL_PEERTOKEN, tok.as_mut_ptr().cast(), &mut len) };
    if rc != 0 || len as usize != std::mem::size_of_val(&tok) {
        return Err(io::Error::last_os_error());
    }
    Ok(tok[5])
}

/// Linux: SO_PEERCRED's pid, as the peer was when it connected.
#[cfg(target_os = "linux")]
pub fn peer_pid(s: &UnixStream) -> io::Result<u32> {
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: cred and len are valid for the call.
    let rc = unsafe {
        libc::getsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut cred as *mut libc::ucred).cast(), &mut len)
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    u32::try_from(cred.pid).map_err(|_| io::Error::other("no peer pid"))
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
pub fn serve(listener: UnixListener, svc: Arc<Mutex<Service>>, runs: Arc<RunRegistry>, allowed_uid: u32) {
    for conn in listener.incoming() {
        let Ok(conn) = conn else { continue };
        let svc = svc.clone();
        let runs = runs.clone();
        std::thread::spawn(move || {
            let _ = handle(conn, &svc, &runs, allowed_uid);
        });
    }
}

fn reply(w: &mut UnixStream, v: &Value) -> io::Result<()> {
    let mut line = v.to_string();
    line.push('\n');
    w.write_all(line.as_bytes())
}

/// Read one line of at most `cap` bytes. `Ok(None)` at EOF,
/// `Err(InvalidData)` if the line is longer than the cap.
fn read_request(r: &mut impl BufRead, cap: usize) -> io::Result<Option<Vec<u8>>> {
    let mut buf = Vec::new();
    r.take(cap as u64 + 1).read_until(b'\n', &mut buf)?;
    if buf.is_empty() {
        return Ok(None);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
    } else if buf.len() > cap {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "request_too_large"));
    }
    Ok(Some(buf))
}

/// Only a `vajra-run` `plan_tasks` request may be longer than `MAX_REQUEST`.
fn within_cap(v: &Value, len: usize) -> bool {
    len <= MAX_REQUEST || (v.get("vajra_run").is_some() && v.get("tool").and_then(Value::as_str) == Some("plan_tasks"))
}

fn handle(mut conn: UnixStream, svc: &Mutex<Service>, runs: &RunRegistry, allowed_uid: u32) -> io::Result<()> {
    match peer_uid(&conn) {
        Ok(uid) if uid == allowed_uid => {}
        _ => return reply(&mut conn, &json!({"error": "peer_uid"})),
    }
    let peer = peer_pid(&conn).ok();
    conn.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut reader = BufReader::new(conn.try_clone()?);
    loop {
        let req = match read_request(&mut reader, runreg::MAX_PLAN_REQUEST) {
            Ok(Some(r)) => r,
            Ok(None) => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                return reply(&mut conn, &json!({"error": "request_too_large"}));
            }
            Err(e) => return Err(e),
        };
        let out = match serde_json::from_slice::<Value>(&req) {
            Ok(v) if !within_cap(&v, req.len()) => json!({"error": "request_too_large"}),
            // A run lead's call: never into the Service lock.
            Ok(v) if v.get("vajra_run").is_some() => runs.serve(&v, peer),
            Ok(v) => match v.get("tool").and_then(Value::as_str) {
                Some(tool) => {
                    let args = v.get("args").cloned().unwrap_or(Value::Null);
                    let mut s = crate::lock(svc);
                    let out = match Binding::from_request(&v) {
                        Err(field) => Err(super::service::ControlError::InvalidArgs(field.into())),
                        Ok(Binding::None) => s.call_tool_for_run(None, tool, &args),
                        Ok(Binding::Run(r)) => s.call_tool_for_run(Some(&r), tool, &args),
                        Ok(Binding::Project(p, g)) => s.call_tool_for_project(&p, g, tool, &args),
                    };
                    match out {
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

/// A run lead's `--mcp-run` client: one `vajra-run` request, one reply line, waiting at most `timeout` for it.
pub fn request_run(path: &Path, run: &str, token: &str, tool: &str, args: &Value, timeout: Duration) -> io::Result<Value> {
    let req = json!({"vajra_run": run, "token": token, "tool": tool, "args": args});
    let mut line = req.to_string();
    line.push('\n');
    let cap = if tool == "plan_tasks" { runreg::MAX_PLAN_REQUEST } else { MAX_REQUEST };
    if line.len() > cap + 1 {
        return Ok(json!({"error": "request_too_large"}));
    }
    let mut s = UnixStream::connect(path)?;
    s.set_read_timeout(Some(timeout))?;
    s.write_all(line.as_bytes())?;
    let mut r = BufReader::new(s);
    let mut out = String::new();
    r.read_line(&mut out)?;
    serde_json::from_str(&out).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Client side: one request, one reply line.
pub fn request(path: &Path, tool: &str, args: &Value) -> io::Result<Value> {
    request_for(path, None, tool, args)
}

/// What a tool call is bound to: nothing (a human-side client), one run (an ADR-13 Lead) or one project's
/// Director-General at one conversation generation (ADR-15 2.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Binding {
    None,
    Run(String),
    Project(String, u32),
}

impl Binding {
    /// The binding a request names. `run` and `project` together, a `project` without its `generation` (or the other
    /// way round), or a value of the wrong type is a bad request, never "unbound". A key set to JSON null is present
    /// with a bad value, not omitted (#81 review): only a request with none of the three keys is unbound.
    fn from_request(v: &Value) -> Result<Self, &'static str> {
        let present = |k: &str| v.get(k).is_some();
        match (present("run"), present("project"), present("generation")) {
            (false, false, false) => Ok(Self::None),
            (true, false, false) => v["run"].as_str().map(|r| Self::Run(r.into())).ok_or("run"),
            (false, true, true) => {
                let p = v["project"].as_str().ok_or("project")?;
                let g = v["generation"].as_u64().and_then(|g| u32::try_from(g).ok()).ok_or("generation")?;
                Ok(Self::Project(p.into(), g))
            }
            _ => Err("binding"),
        }
    }
}

/// Like `request`, bound to `run` when given.
pub fn request_for(path: &Path, run: Option<&str>, tool: &str, args: &Value) -> io::Result<Value> {
    request_bound(path, &run.map_or(Binding::None, |r| Binding::Run(r.into())), tool, args)
}

/// Like `request`, with an explicit binding.
pub fn request_bound(path: &Path, binding: &Binding, tool: &str, args: &Value) -> io::Result<Value> {
    let mut s = UnixStream::connect(path)?;
    s.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut req = json!({"tool": tool, "args": args});
    match binding {
        Binding::None => {}
        Binding::Run(r) => req["run"] = json!(r),
        Binding::Project(p, g) => {
            req["project"] = json!(p);
            req["generation"] = json!(g);
        }
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
