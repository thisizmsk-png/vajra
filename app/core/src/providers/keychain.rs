//! Write-once storage of a pasted API key in the macOS Keychain (spec D3). API keys only, never a CLI's OAuth token (H1).
//!
//! The key goes in once, through `/usr/bin/security -i` on **stdin** (never argv), into a generic password with service
//! `vajra.provider.<account>` and the access list `-T /usr/bin/security`, so the harness's own key command can read it at
//! start without a prompt. Then it is dropped (and zeroed). **There is no read-back:** the `Store` trait can only check
//! that an item exists, add one and delete one, and the only answer is `saved: true|false`. Errors are fixed codes; the
//! child's stdout and stderr are never read, so nothing it prints can reach a reply or a log.

use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The only `security` the app runs. Tests inject a fake through `Cli::new`, never through the environment or IPC.
pub const SECURITY: &str = "/usr/bin/security";
/// The accounts that may hold a stored key. The service name is built from these, never from caller text.
pub const KEY_ACCOUNTS: &[&str] = &["openrouter"];
/// A Keychain unlock prompt can wait on the user; past this the call fails rather than hanging.
const TIMEOUT: Duration = Duration::from_secs(30);
/// `errSecItemNotFound`, as `security` reports it.
const NOT_FOUND: i32 = 44;

/// Why a key operation failed. Fixed codes with fixed words: nothing here can carry input or child output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The account is not one that holds a stored key.
    UnknownAccount,
    /// The pasted text is not key-shaped (length or characters). Refused before any process runs.
    InvalidKey,
    /// Save when a key is already stored; Replace is the only way to overwrite.
    AlreadySaved,
    /// `security` could not be run, timed out, or answered something unexpected.
    Unavailable,
    /// The write ran but no item is there afterwards.
    WriteFailed,
}

impl Error {
    pub fn code(self) -> &'static str {
        match self {
            Error::UnknownAccount | Error::InvalidKey => "invalid_args",
            Error::AlreadySaved => "already_saved",
            Error::Unavailable => "keychain_unavailable",
            Error::WriteFailed => "keychain_write_failed",
        }
    }
    pub fn detail(self) -> &'static str {
        match self {
            Error::UnknownAccount => "account: does not take a stored key",
            Error::InvalidKey => "key: must be 16 to 256 characters of letters, digits and . _ ~ + / = -",
            Error::AlreadySaved => "a key is already saved for this account; use Replace",
            Error::Unavailable => "the Keychain did not answer",
            Error::WriteFailed => "the Keychain did not keep the key",
        }
    }
}

/// A pasted key. Not `Clone`, no `Display`, a redacting `Debug`, zeroed on drop. Only this module sees its bytes.
pub struct Key(Vec<u8>);

impl Key {
    /// Check the shape and take ownership (the string's buffer is moved, not copied). A value that could break the
    /// `security -i` line (whitespace, quotes, backslash, `#`, a leading `-`) is refused here.
    pub fn new(s: String) -> Result<Key, Error> {
        let k = Key(s.into_bytes());
        let ok = (16..=256).contains(&k.0.len())
            && k.0[0] != b'-'
            && k.0.iter().all(|b| b.is_ascii_alphanumeric() || b"._~+/=-".contains(b));
        if ok { Ok(k) } else { Err(Error::InvalidKey) }
    }
    fn bytes(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key(redacted)")
    }
}

fn zero(v: &mut [u8]) {
    for b in v.iter_mut() {
        // SAFETY: a valid, exclusively borrowed byte; volatile so the clearing is not optimised away.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

impl Drop for Key {
    fn drop(&mut self) {
        zero(&mut self.0);
    }
}

/// The Keychain as the policy needs it. There is deliberately no way to read a stored value back.
pub trait Store: Send + Sync {
    fn exists(&self, service: &str, account: &str) -> Result<bool, Error>;
    fn add(&self, service: &str, account: &str, key: &Key) -> Result<(), Error>;
    /// `Ok(false)` when there was nothing to delete.
    fn delete(&self, service: &str, account: &str) -> Result<bool, Error>;
}

/// The real store: `/usr/bin/security` (or an injected fake in tests), with a cleared environment.
pub struct Cli {
    bin: PathBuf,
    home: PathBuf,
}

impl Cli {
    pub fn new(bin: PathBuf, home: PathBuf) -> Self {
        Self { bin, home }
    }

    /// Run `security args...`, feeding `stdin` if given. Only the exit code is used; stdout and stderr go nowhere.
    fn status(&self, args: &[&str], stdin: Option<&[u8]>) -> Result<i32, Error> {
        let mut cmd = Command::new(&self.bin);
        cmd.args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.home)
            .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        let mut child = cmd.spawn().map_err(|_| Error::Unavailable)?;
        if let Some(bytes) = stdin {
            // The pipe is dropped at the end of the closure: EOF, so `security -i` exits after the line.
            if !child.stdin.take().is_some_and(|mut pipe| pipe.write_all(bytes).is_ok()) {
                kill(&mut child);
                return Err(Error::Unavailable);
            }
        }
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(s)) => return s.code().ok_or(Error::Unavailable),
                Ok(None) if start.elapsed() < TIMEOUT => std::thread::sleep(Duration::from_millis(20)),
                _ => {
                    kill(&mut child);
                    return Err(Error::Unavailable);
                }
            }
        }
    }
}

fn kill(child: &mut std::process::Child) {
    // SAFETY: killpg on the group this child leads (`process_group(0)`); ESRCH is fine.
    unsafe { libc::killpg(child.id() as i32, libc::SIGKILL) };
    let _ = child.wait();
}

impl Store for Cli {
    fn exists(&self, service: &str, account: &str) -> Result<bool, Error> {
        // No `-w` or `-g`: the password data is never asked for, so this cannot prompt or print it.
        match self.status(&["find-generic-password", "-s", service, "-a", account], None)? {
            0 => Ok(true),
            NOT_FOUND => Ok(false),
            _ => Err(Error::Unavailable),
        }
    }

    fn add(&self, service: &str, account: &str, key: &Key) -> Result<(), Error> {
        // The whole command goes through stdin so the key is never in argv. `service` and `account` come from
        // `KEY_ACCOUNTS` and `key` passed `Key::new`, so no token can break the line. The exit code of `security -i` is
        // not trusted to reflect the command; the caller checks `exists` afterwards.
        let head = format!("add-generic-password -a {account} -s {service} -T {SECURITY} -w ");
        let mut line = Vec::with_capacity(head.len() + key.bytes().len() + 1);
        line.extend_from_slice(head.as_bytes());
        line.extend_from_slice(key.bytes());
        line.push(b'\n');
        let r = self.status(&["-i"], Some(&line));
        zero(&mut line);
        r.map(|_| ())
    }

    fn delete(&self, service: &str, account: &str) -> Result<bool, Error> {
        match self.status(&["delete-generic-password", "-s", service, "-a", account], None)? {
            0 => Ok(true),
            NOT_FOUND => Ok(false),
            _ => Err(Error::Unavailable),
        }
    }
}

/// The write-once policy over a `Store`. One lock covers each check and write, so two Saves cannot both pass the check.
pub struct Keys {
    store: Box<dyn Store>,
    lock: Mutex<()>,
}

/// The `security` service name for an account that takes a stored key.
pub(crate) fn service(account: &str) -> Result<String, Error> {
    if KEY_ACCOUNTS.contains(&account) { Ok(format!("vajra.provider.{account}")) } else { Err(Error::UnknownAccount) }
}

impl Keys {
    pub fn new(store: Box<dyn Store>) -> Self {
        Self { store, lock: Mutex::new(()) }
    }

    /// The real Keychain through `bin` (`SECURITY` outside tests).
    pub fn cli(bin: &Path, home: &Path) -> Self {
        Self::new(Box::new(Cli::new(bin.to_path_buf(), home.to_path_buf())))
    }

    fn guard(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Whether a key is stored. The only thing the core can tell about a key.
    pub fn saved(&self, account: &str) -> Result<bool, Error> {
        let s = service(account)?;
        let _g = self.guard();
        self.store.exists(&s, account)
    }

    /// Store a key for the first time; refused with `AlreadySaved` when one is there.
    pub fn save(&self, account: &str, key: Key) -> Result<(), Error> {
        let s = service(account)?;
        let _g = self.guard();
        if self.store.exists(&s, account)? {
            return Err(Error::AlreadySaved);
        }
        self.write(&s, account, key)
    }

    /// Overwrite (or first store) a key: delete, then add, so the new item gets a fresh access list.
    pub fn replace(&self, account: &str, key: Key) -> Result<(), Error> {
        let s = service(account)?;
        let _g = self.guard();
        self.store.delete(&s, account)?;
        self.write(&s, account, key)
    }

    /// Delete the stored key. Removing a key that is not there is not an error.
    pub fn remove(&self, account: &str) -> Result<(), Error> {
        let s = service(account)?;
        let _g = self.guard();
        self.store.delete(&s, account).map(|_| ())
    }

    fn write(&self, service: &str, account: &str, key: Key) -> Result<(), Error> {
        self.store.add(service, account, &key)?;
        drop(key);
        if self.store.exists(service, account)? { Ok(()) } else { Err(Error::WriteFailed) }
    }
}

#[cfg(test)]
#[path = "keychain_tests.rs"]
pub(crate) mod tests;
