//! H9 end to end against a local stub that returns 429, 504 or hangs: a Rust port of the spike's `stub.py`. The client
//! is a plain HTTP/1.1 POST over a loopback socket; its failure text is shaped like Codex's, and the retry waits run on
//! a fake clock, so the test takes milliseconds. No real key, HOME or network is touched.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use vajra_desktop::providers::errors::{Action, FakeClock, Ident, Kind, Outcome, ProviderError, run_with_policy};

#[derive(Clone, Copy)]
enum Mode {
    TooMany,
    GatewayTimeout,
    Hang,
}

struct Stub {
    addr: SocketAddr,
    hits: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
}

impl Stub {
    fn start(mode: Mode) -> Stub {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (hits, stop) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicBool::new(false)));
        let (h, s) = (hits.clone(), stop.clone());
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut c) = conn else { break };
                if s.load(Ordering::SeqCst) {
                    break;
                }
                h.fetch_add(1, Ordering::SeqCst);
                let s = s.clone();
                std::thread::spawn(move || serve(&mut c, mode, &s));
            }
        });
        Stub { addr, hits, stop }
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
    }
}

fn serve(c: &mut TcpStream, mode: Mode, stop: &AtomicBool) {
    let mut buf = [0u8; 4096];
    let _ = c.read(&mut buf);
    let (code, reason, msg, retry_after) = match mode {
        Mode::Hang => {
            // Headers, then silence: the client's idle timeout is the only way out.
            let _ = c.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n");
            while !stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(20));
            }
            return;
        }
        Mode::TooMany => (429, "Too Many Requests", "Rate limit exceeded: free-models-per-min", Some(7)),
        Mode::GatewayTimeout => (504, "Gateway Timeout", "Upstream idle timeout exceeded", None),
    };
    let body = format!("{{\"error\":{{\"code\":{code},\"message\":\"{msg}\"}}}}");
    let extra = retry_after.map_or(String::new(), |s| format!("Retry-After: {s}\r\n"));
    let _ = write!(c, "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\n\r\n{body}", body.len());
}

/// One POST. A non-2xx answer or a silent stream becomes the failure text Codex would print.
fn attempt(addr: SocketAddr, idle: Duration) -> Result<(), String> {
    let mut c = TcpStream::connect(addr).map_err(|e| e.to_string())?;
    c.set_read_timeout(Some(idle)).unwrap();
    c.write_all(b"POST /v1/responses HTTP/1.1\r\nHost: stub\r\nContent-Length: 2\r\n\r\n{}").unwrap();
    let mut got = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        match c.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                got.extend_from_slice(&buf[..n]);
                if got.windows(4).any(|w| w == b"\r\n\r\n") && !got.starts_with(b"HTTP/1.1 200") {
                    std::thread::sleep(Duration::from_millis(20));
                    let _ = c.read(&mut buf).map(|n| got.extend_from_slice(&buf[..n]));
                    break;
                }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return Err("idle timeout waiting for SSE".into()),
            Err(e) => return Err(e.to_string()),
        }
    }
    let text = String::from_utf8_lossy(&got).to_string();
    let status: u16 = text.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    if status == 200 {
        return Ok(());
    }
    let retry = text.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("retry-after:").map(|v| v.trim().to_string()));
    let body = text.split("\r\n\r\n").nth(1).unwrap_or("");
    Err(format!("unexpected status {status}: {body}{}", retry.map_or(String::new(), |r| format!(" retry-after: {r}"))))
}

fn run(mode: Mode) -> (Outcome, Vec<ProviderError>, Vec<Duration>, usize) {
    let stub = Stub::start(mode);
    let clock = FakeClock::default();
    let who = Ident { slot: "w1", session_id: "s_fixture", account: "gateway-fixture" };
    let mut events = Vec::new();
    let out = run_with_policy(&clock, &who, &mut |e| events.push(e.clone()), || attempt(stub.addr, Duration::from_millis(150)));
    (out, events, clock.slept(), stub.hits())
}

#[test]
fn a_429_with_retry_after_waits_the_window_then_asks_the_user() {
    let (out, ev, slept, hits) = run(Mode::TooMany);
    assert_eq!(hits, 3, "one try and two waits");
    assert_eq!(slept, vec![Duration::from_secs(7), Duration::from_secs(7)]);
    assert!(ev.iter().all(|e| e.kind == Kind::RateLimit && e.http_status == Some(429) && e.retry_after_s == Some(7)));
    assert!(matches!(out, Outcome::Blocked(e) if e.action == Action::AskingUser && e.attempt == 3));
}

#[test]
fn a_504_is_retried_twice_with_backoff_then_asks_the_user() {
    let (out, ev, slept, hits) = run(Mode::GatewayTimeout);
    assert_eq!(hits, 3);
    assert_eq!(slept, vec![Duration::from_secs(30), Duration::from_secs(120)]);
    assert_eq!(ev.iter().map(|e| e.action).collect::<Vec<_>>(), vec![Action::Retrying, Action::Retrying, Action::AskingUser]);
    assert!(ev.iter().all(|e| e.kind == Kind::Timeout && e.account == "gateway-fixture"));
    assert!(matches!(out, Outcome::Blocked(_)));
}

#[test]
fn a_hung_stream_times_out_is_retried_twice_then_asks_the_user() {
    let (out, ev, slept, hits) = run(Mode::Hang);
    assert_eq!(hits, 3);
    assert_eq!(slept, vec![Duration::from_secs(30), Duration::from_secs(120)]);
    assert!(ev.iter().all(|e| e.kind == Kind::Timeout && e.http_status.is_none()));
    assert!(matches!(out, Outcome::Blocked(_)));
}
