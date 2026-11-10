//! Usage meters (`vajra.provider_usage/1`, H5). What an account has used, as the account's own source reports it, or
//! "Not reported". Nothing is estimated: there is no price table, and Claude Code's `total_cost_usd` is ignored.
//!
//! - A subscription account reports windows only if its CLI does. None of the status commands do today, so those accounts
//!   read `source: "none"` with no windows.
//! - A gateway key (OpenRouter) is read with `GET /api/v1/key`. That is an outbound call carrying a key, so:
//!   - it runs only while Settings > Providers is open or a run is active, and at most once a minute per account
//!     (`Meter`); a project-state read only ever returns the last answer;
//!   - the key is never in this process: a key command prints it straight into curl's stdin (`CurlSource`), so it is
//!     never in argv, a log, a reply or a `Debug` output. There is no getter for it anywhere;
//!   - only `usage`, `usage_daily`, `limit` and `limit_remaining` are kept. The response also carries the key's label,
//!     which is never deserialized, so it cannot reach an answer.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::keychain::{self, Keys, KEY_ACCOUNTS};

pub const USAGE_SCHEMA: &str = "vajra.provider_usage/1";
/// At this percentage used a window carries `warn`.
pub const WARN_PCT: u8 = 80;
/// The least time between two outbound reads for one account.
pub const MIN_INTERVAL: Duration = Duration::from_secs(60);
const KEY_URL: &str = "https://openrouter.ai/api/v1/key";
const CALL_DEADLINE: Duration = Duration::from_secs(15);
const MAX_RESPONSE_BYTES: u64 = 64 * 1024;

/// One window of use. `resets_at` is present only if the source gave one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Window {
    /// `5h`, `daily`, `weekly` or `credit` (a key's own spending limit).
    pub kind: String,
    /// 0 to 100.
    pub used_pct: u8,
    #[serde(default)]
    pub resets_at: Option<String>,
    /// `used_pct` is at or above `WARN_PCT`.
    pub warn: bool,
}

impl Window {
    pub fn new(kind: &str, used_pct: f64, resets_at: Option<String>) -> Window {
        let pct = if used_pct.is_finite() { used_pct.clamp(0.0, 100.0).round() as u8 } else { 0 };
        Window { kind: kind.into(), used_pct: pct, resets_at, warn: pct >= WARN_PCT }
    }
}

/// The four figures the key endpoint reports, in the account's own currency. Reported, never estimated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Amounts {
    pub usage: f64,
    #[serde(default)]
    pub usage_daily: Option<f64>,
    #[serde(default)]
    pub limit: Option<f64>,
    #[serde(default)]
    pub limit_remaining: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub account: String,
    /// When this was read; `None` for "Not reported".
    #[serde(default)]
    pub read_at: Option<String>,
    /// `none` (nothing reported) or `gateway_key`.
    pub source: String,
    /// Empty when nothing is reported; the UI says "Not reported".
    pub windows: Vec<Window>,
    #[serde(default)]
    pub amounts: Option<Amounts>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageList {
    pub schema: String,
    pub usage: Vec<Usage>,
}

impl UsageList {
    /// What the figures say, without when they were read: two lists with the same fingerprint show the user the same
    /// thing, so a re-read that only moves `read_at` is not a change.
    pub fn fingerprint(&self) -> String {
        let figures: Vec<_> = self.usage.iter().map(|u| Usage { read_at: None, ..u.clone() }).collect();
        serde_json::to_string(&figures).unwrap_or_default()
    }
}

impl Usage {
    pub fn not_reported(account: &str) -> Usage {
        Usage { account: account.into(), read_at: None, source: "none".into(), windows: Vec::new(), amounts: None }
    }
}

/// Parse a key-endpoint answer, keeping only the four numeric fields. `None` when it is not the expected shape, a figure
/// is negative or not finite, or `usage` is missing: that reads as "Not reported".
pub fn parse_key_response(account: &str, bytes: &[u8], read_at: &str) -> Option<Usage> {
    // Only these four fields exist here. The label and anything else in the answer is never deserialized.
    #[derive(Deserialize)]
    struct Data {
        usage: Option<f64>,
        usage_daily: Option<f64>,
        limit: Option<f64>,
        limit_remaining: Option<f64>,
    }
    #[derive(Deserialize)]
    struct Envelope {
        data: Data,
    }
    if bytes.len() as u64 > MAX_RESPONSE_BYTES {
        return None;
    }
    let d = serde_json::from_slice::<Envelope>(bytes).ok()?.data;
    let ok = |v: Option<f64>| v.is_none_or(|n| n.is_finite() && n >= 0.0);
    if ![d.usage, d.usage_daily, d.limit, d.limit_remaining].into_iter().all(ok) {
        return None;
    }
    let usage = d.usage?;
    // `usage` is all-time spend, while `limit` can reset daily, weekly or monthly and `limit_remaining` is the current cap's
    // room left. So the current window's use is `limit - limit_remaining`; without both figures there is no percentage,
    // and none is made up from `usage`.
    let windows = match (d.limit, d.limit_remaining) {
        (Some(limit), Some(left)) if limit > 0.0 => vec![Window::new("credit", (limit - left) / limit * 100.0, None)],
        _ => Vec::new(),
    };
    Some(Usage {
        account: account.into(),
        read_at: Some(read_at.into()),
        source: "gateway_key".into(),
        windows,
        amounts: Some(Amounts { usage, usage_daily: d.usage_daily, limit: d.limit, limit_remaining: d.limit_remaining }),
    })
}

/// Where metered accounts and their key-endpoint answers come from. The real source is `CurlSource` over `KeychainKeys`
/// (the Keychain store); `NoSource` reports nothing.
pub trait UsageSource: Send + Sync {
    /// Accounts that have a key to meter.
    fn accounts(&self) -> Vec<String>;
    /// The raw answer of the key endpoint for `account`, or `None` when it could not be read.
    fn key_info(&self, account: &str) -> Option<Vec<u8>>;
}

/// No key is stored yet: no metered account, no call.
pub struct NoSource;

impl UsageSource for NoSource {
    fn accounts(&self) -> Vec<String> {
        Vec::new()
    }
    fn key_info(&self, _account: &str) -> Option<Vec<u8>> {
        None
    }
}

/// The command that prints an account's key on its stdout (for example `security find-generic-password -s
/// vajra.provider.<account> -w`). It is run only as the producer of curl's stdin; its output never enters this process.
pub trait KeyCommands: Send + Sync {
    fn accounts(&self) -> Vec<String>;
    fn argv(&self, account: &str) -> Option<Vec<String>>;
    /// The `HOME` the key command runs with (the Keychain lookup needs the user's home), when it needs one.
    fn home(&self) -> Option<&Path> {
        None
    }
}

/// The accounts whose key is stored in the Keychain (PR 2), and the `security` command that prints one on stdout. The
/// core never reads the key: this only names the command, which `CurlSource` runs as the producer of curl's stdin.
pub struct KeychainKeys {
    keys: Keys,
    security: PathBuf,
    home: PathBuf,
    /// The last answer of `accounts`, so a burst of `project_state` reads runs `security` once, not once each.
    listed: Mutex<Option<(Instant, Vec<String>)>>,
}

/// How long a list of stored-key accounts is reused. Short, so a key saved on the Providers page shows up at once.
const LIST_TTL: Duration = Duration::from_secs(5);

impl KeychainKeys {
    pub fn new(security: &Path, home: &Path) -> Self {
        Self { keys: Keys::cli(security, home), security: security.to_path_buf(), home: home.to_path_buf(), listed: Mutex::new(None) }
    }
}

impl KeyCommands for KeychainKeys {
    /// Accounts with a stored key. `saved` asks `security` for the item without `-w`, so this never prints a key.
    fn accounts(&self) -> Vec<String> {
        let mut listed = self.listed.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((_, accounts)) = listed.as_ref().filter(|(at, _)| at.elapsed() < LIST_TTL) {
            return accounts.clone();
        }
        let accounts: Vec<String> = KEY_ACCOUNTS.iter().filter(|a| self.keys.saved(a) == Ok(true)).map(|a| a.to_string()).collect();
        *listed = Some((Instant::now(), accounts.clone()));
        accounts
    }
    /// `[security, find-generic-password, -s, vajra.provider.<a>, -a, <a>, -w]`: the account must be a key account.
    fn argv(&self, account: &str) -> Option<Vec<String>> {
        let service = keychain::service(account).ok()?;
        let security = self.security.to_str()?;
        Some([security, "find-generic-password", "-s", &service, "-a", account, "-w"].map(String::from).into())
    }
    fn home(&self) -> Option<&Path> {
        Some(&self.home)
    }
}

/// `GET /api/v1/key` through curl, with the key piped from its key command into curl's stdin as a header file (`-H @-`).
/// The key is never in argv, the environment, a file or this process's memory. The environment is cleared.
pub struct CurlSource {
    curl: PathBuf,
    url: String,
    deadline: Duration,
    keys: Box<dyn KeyCommands>,
}

impl CurlSource {
    pub fn new(keys: Box<dyn KeyCommands>) -> Self {
        Self { curl: PathBuf::from("/usr/bin/curl"), url: KEY_URL.into(), deadline: CALL_DEADLINE, keys }
    }
    /// Tests point this at a fake curl.
    #[cfg(test)]
    fn with_curl(mut self, curl: PathBuf) -> Self {
        self.curl = curl;
        self
    }
    #[cfg(test)]
    fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }
}

impl UsageSource for CurlSource {
    fn accounts(&self) -> Vec<String> {
        self.keys.accounts()
    }

    fn key_info(&self, account: &str) -> Option<Vec<u8>> {
        let key_cmd = self.keys.argv(account)?;
        // sh prints the header name, then becomes the key command: its stdout continues the same pipe into curl.
        let mut producer = Command::new("/bin/sh");
        producer
            .args(["-c", "printf 'Authorization: Bearer '; exec \"$@\"", "sh"])
            .args(&key_cmd)
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin");
        if let Some(home) = self.keys.home() {
            producer.env("HOME", home);
        }
        let mut producer = producer
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let pipe = producer.stdout.take()?;
        let curl = Command::new(&self.curl)
            // -q first: never read a .curlrc (it could add a proxy or another header to a request that carries a key).
            .args(["-q", "-sS", "--fail", "--proto", "=https", "--max-time", "10", "-H", "@-", "--", &self.url])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::from(pipe))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let mut curl = match curl {
            Ok(c) => c,
            Err(_) => {
                let _ = producer.kill();
                let _ = producer.wait();
                return None;
            }
        };
        let mut out = curl.stdout.take()?;
        let reader = std::thread::spawn(move || {
            let mut body = Vec::new();
            out.by_ref().take(MAX_RESPONSE_BYTES + 1).read_to_end(&mut body).ok().map(|_| body)
        });
        let start = Instant::now();
        let status = loop {
            match curl.try_wait() {
                Ok(Some(s)) => break Some(s),
                Ok(None) if start.elapsed() < self.deadline => std::thread::sleep(Duration::from_millis(20)),
                _ => {
                    let _ = curl.kill();
                    let _ = curl.wait();
                    break None;
                }
            }
        };
        let _ = producer.kill();
        let _ = producer.wait();
        let body = reader.join().ok().flatten();
        (status?.success()).then_some(body).flatten()
    }
}

/// The last answer per account, and the rate limit and activity rule around reading a new one.
#[derive(Default)]
pub struct Meter {
    last: Mutex<HashMap<String, (Instant, Usage)>>,
}

impl Meter {
    /// The usage for `account`. A fresh enough answer (under `MIN_INTERVAL`) is returned as is. When `active` is false (the
    /// Providers page is closed and no run is going) nothing is read: the last answer, or "Not reported", is returned. A
    /// failed read keeps the previous answer (its `read_at` shows its age) and still counts as an attempt.
    pub fn read(&self, account: &str, active: bool, now: Instant, read_at: &str, source: &dyn UsageSource) -> Usage {
        {
            let mut last = lock(&self.last);
            match last.get_mut(account) {
                Some((at, u)) => {
                    if !active || now.saturating_duration_since(*at) < MIN_INTERVAL {
                        return u.clone();
                    }
                    // Claim the attempt before the call, so a second caller meanwhile reads the cache.
                    *at = now;
                }
                None if !active => return Usage::not_reported(account),
                None => {
                    last.insert(account.into(), (now, Usage::not_reported(account)));
                }
            }
        }
        // The call runs without the lock: a slow endpoint must not stall readers of the cache.
        let fresh = source.key_info(account).and_then(|b| parse_key_response(account, &b, read_at));
        let mut last = lock(&self.last);
        let entry = last.entry(account.into()).or_insert_with(|| (now, Usage::not_reported(account)));
        if let Some(u) = fresh {
            entry.1 = u;
        }
        entry.1.clone()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
