//! Plan and run a one-click engine install (D7 design E5-E7; bhishma C3, C4, C5, C7, C8, C9, C10).
//!
//! The plan is read-only. The run holds the install lock, recomputes the plan and requires the same hash, then:
//! stage the verified bytes, write the workflows itself, swap in, re-hash the tree, run the engine's own installer with
//! `--no-hooks` (so Claude Code's settings are never read or written), re-hash again and require the manifest check to
//! pass, then record and commit. Any failure rolls back exactly; a rollback it can't confirm marks the engine Broken.
//! The app never opens the signing key: it checks it by `lstat` only.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::bundle::{self, Engine};
use super::fsat::{Dir, Kind};
use super::place::{self, Found, Place, SKILL};
use super::workflows;
use super::Pin as BuildPin;
use crate::providers::install::plan::{Pin, Plan};
use crate::providers::install::run::{spawn_wait, Ctx, Outcome, Report};

/// The engine's catalog id and its Broken-mark id.
pub const ID: &str = "engine";
/// The engine installer, relative to the skill folder.
pub const INSTALLER: &str = "scripts/install.sh";
/// The manifest check, relative to the skill folder: exit 0 means every listed file matches and the signature is good.
pub const CHECK: &str = "hooks/session-start.sh";
/// The engine's own timeout (C10), not the 15 minutes a package manager gets.
pub const TIMEOUT: Duration = Duration::from_secs(180);
/// The root-owned folders the engine run's PATH is (C7): never `/opt/homebrew/bin` or `/usr/local/bin`.
pub const ENGINE_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";
/// The signing key's file name, checked by `lstat` only.
const KEY: &str = ".hmac-key";
const KEY_TMP: &str = ".hmac-key.tmp";

/// The tools the engine installer needs, at the root-owned paths it and the hooks use (no Homebrew fallback).
pub struct Tools {
    pub bash: PathBuf,
    pub jq: PathBuf,
    pub openssl: PathBuf,
    /// Whether the command line tools are installed (`/usr/bin/python3` and git are stubs without them).
    pub clt: bool,
}

impl Tools {
    pub fn system() -> Tools {
        let clt = std::process::Command::new("/usr/bin/xcode-select").arg("-p").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
        Tools { bash: "/bin/bash".into(), jq: "/usr/bin/jq".into(), openssl: "/usr/bin/openssl".into(), clt }
    }

    /// The first missing tool, as a manual reason.
    fn missing(&self) -> Option<&'static str> {
        let ok = |p: &Path| std::fs::metadata(p).is_ok_and(|m| m.is_file());
        if !ok(&self.bash) {
            return Some("engine_needs_bash");
        }
        if !ok(&self.jq) {
            return Some("engine_needs_jq");
        }
        if !ok(&self.openssl) {
            return Some("engine_needs_openssl");
        }
        None
    }
}

/// What a plan needs that isn't on disk: the user's home, app data, the app's resources, the build pin and tools.
pub struct Machine<'a> {
    pub home: &'a Path,
    pub data: &'a Path,
    pub resources: &'a Path,
    pub pin: &'a BuildPin,
    pub tools: &'a Tools,
    pub uid: u32,
    /// Whether anything is using the engine right now (an active run, a live session, an open campaign): C8.
    pub busy: &'a dyn Fn() -> bool,
}

/// The engine's offer: a plan, or the reason it can't be offered (a manual reason), with notes either way.
pub enum Offer {
    Plan(Box<Prepared>),
    Manual(&'static str),
}

/// A plan and what the run needs to carry it out.
pub struct Prepared {
    pub plan: Plan,
    pub engine: Engine,
    pub found: Found,
    pub want: BTreeMap<String, Vec<u8>>,
}

/// Plan the install. Read-only: it reads the bundle, the target and the workflows, and writes nothing.
pub fn plan(m: &Machine) -> Offer {
    if (m.busy)() {
        return Offer::Manual("engine_in_use");
    }
    if let Some(t) = m.tools.missing() {
        return Offer::Manual(t);
    }
    let engine = match bundle::load(m.resources, m.pin) {
        Ok(e) => e,
        Err(r) => return Offer::Manual(r.code()),
    };
    let Ok(place) = Place::open_existing(m.home, m.data, m.uid) else { return Offer::Manual("not_writable") };
    if let Err(r) = state_safe(m.home, m.uid) {
        return Offer::Manual(r);
    }
    let found = place.found();
    let (case, old_version, old_root) = match &found {
        Found::Absent => ("fresh", None, None),
        Found::Ours { version, dev, ino } => (if *version == engine.version { "repair" } else if newer(version, &engine.version) { "downgrade" } else { "update" }, Some(version.clone()), Some((*dev, *ino))),
        Found::UserCopy => return Offer::Manual("engine_user_copy"),
    };
    if !place.has_space(m.pin.unpacked) {
        return Offer::Manual("no_space");
    }
    let want = workflows::shipped(&engine);
    let snap = match place.workflows_snapshot(want.keys().cloned()) {
        Ok(s) => s,
        Err(r) => return Offer::Manual(r),
    };
    let mut notes = vec!["no_hooks", "copies_workflows"];
    if !key_exists(m.home) {
        notes.push("creates_key");
    }
    if m.home.join(".claude/vajra/.integrity-violation").exists() {
        notes.push("clears_integrity_lock");
    }
    if place.journal().is_some() {
        notes.push("recovers_previous");
    }
    if !m.tools.clt {
        notes.push("python3_missing");
    }
    if !place.strays().is_empty() {
        notes.push("leftovers_kept");
    }
    let skill = m.home.join(".claude/skills").join(SKILL);
    let command = vec![m.tools.bash.display().to_string(), skill.join(INSTALLER).display().to_string(), "--no-hooks".into()];
    let build = if m.pin.release { "release" } else { "development" };
    let pin = Pin::Engine {
        tgz_sha256: m.pin.sha256.clone(),
        build,
        case,
        old_version,
        old_root,
        workflows: workflows::changes(&snap, &want),
        home: m.home.display().to_string(),
    };
    let source = if m.pin.release { "Bundled with Vajra Desktop".to_string() } else { "Bundled with this development build of Vajra Desktop".to_string() };
    let mut plan = Plan::new(ID, "engine_bundle", command, Vec::new(), source, engine.version.clone(), skill.display().to_string(), pin);
    plan.notes = notes;
    Offer::Plan(Box::new(Prepared { plan: plan.rehash(), engine, found, want }))
}

/// Whether version `a` is newer than `b` (major.minor.patch, numerically).
fn newer(a: &str, b: &str) -> bool {
    let p = |v: &str| v.split(['.', '-', '+']).take(3).map(|n| n.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    p(a) > p(b)
}

fn key_exists(home: &Path) -> bool {
    std::fs::symlink_metadata(home.join(".claude/vajra").join(KEY)).is_ok()
}

/// bhishma C4: the state folder and the signing key are safe for the installer to touch. Checked by `lstat` only;
/// the key is never opened. A planted temp key (which the installer would follow), a key that isn't a single regular
/// file of 64 or 65 bytes (which the installer would replace), or a state folder that isn't the user's own 0700 folder
/// are each refused.
fn state_safe(home: &Path, uid: u32) -> Result<(), &'static str> {
    let vajra = home.join(".claude/vajra");
    use std::os::unix::fs::MetadataExt;
    match std::fs::symlink_metadata(&vajra) {
        Err(_) => return Ok(()),
        Ok(m) if m.file_type().is_dir() && m.uid() == uid && m.mode() & 0o077 == 0 => {}
        Ok(_) => return Err("engine_state_unsafe"),
    }
    if std::fs::symlink_metadata(vajra.join(KEY_TMP)).is_ok() {
        return Err("engine_state_unsafe");
    }
    match std::fs::symlink_metadata(vajra.join(KEY)) {
        Err(_) => Ok(()),
        Ok(m) if m.file_type().is_file() && m.nlink() == 1 && (64..=65).contains(&m.len()) => Ok(()),
        Ok(_) => Err("engine_state_unsafe"),
    }
}

/// After the run: the key is a single regular file of the user's, mode 600.
fn key_safe(home: &Path, uid: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(home.join(".claude/vajra").join(KEY)).is_ok_and(|m| m.file_type().is_file() && m.uid() == uid && m.mode() & 0o777 == 0o600 && m.nlink() == 1)
}

/// The signature and tamper-flag state, restored exactly on rollback (C5).
struct SigState {
    sig: Option<Vec<u8>>,
    flag: bool,
}

fn sig_state(home: &Path) -> SigState {
    let v = home.join(".claude/vajra");
    SigState { sig: std::fs::read(v.join("manifest.sig")).ok(), flag: v.join(".integrity-violation").exists() }
}

fn restore_sig(home: &Path, s: &SigState) {
    let v = home.join(".claude/vajra");
    match &s.sig {
        Some(b) => {
            let tmp = v.join("manifest.sig.vajra-restore");
            let _ = std::fs::remove_file(&tmp);
            use std::os::unix::fs::OpenOptionsExt;
            if let Ok(mut f) = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(&tmp) {
                if std::io::Write::write_all(&mut f, b).is_ok() {
                    let _ = std::fs::rename(&tmp, v.join("manifest.sig"));
                }
            }
        }
        None => {
            let _ = std::fs::remove_file(v.join("manifest.sig"));
        }
    }
    let flag = v.join(".integrity-violation");
    if s.flag && !flag.exists() {
        let _ = std::fs::write(&flag, b"");
    } else if !s.flag {
        let _ = std::fs::remove_file(&flag);
    }
}

/// Re-hash the installed tree against `engine` (C3): every file present, regular, one link, the right bytes, and no
/// file the engine doesn't have under the folders code runs from.
pub fn tree_matches(root: &Dir, engine: &Engine) -> bool {
    for f in &engine.files {
        let mut parts: Vec<&str> = f.name.split('/').collect();
        let Some(file) = parts.pop() else { return false };
        let Ok(dir) = root.open_dir_path(&parts) else { return false };
        if dir.read(file, 64 << 20).ok().as_deref() != Some(f.bytes.as_slice()) {
            return false;
        }
    }
    // No extra files where code is loaded from.
    for top in ["hooks", "scripts", "config", "workflows", "atman"] {
        let Ok(d) = root.open_dir(top) else { continue };
        if extra(&d, top, engine) {
            return false;
        }
    }
    true
}

fn extra(d: &Dir, prefix: &str, engine: &Engine) -> bool {
    let Ok(names) = d.list() else { return true };
    names.into_iter().any(|n| {
        let path = format!("{prefix}/{n}");
        match d.stat_at(&n) {
            Ok(Some(s)) if s.kind == Kind::Dir => d.open_dir(&n).map_or(true, |sub| extra(&sub, &path, engine)),
            Ok(Some(s)) if s.kind == Kind::File => engine.get(&path).is_none(),
            _ => true,
        }
    })
}

/// Hold the install lock (C8): an exclusive `flock` on `~/.claude/.vajra-install/lock`, so two app instances never
/// install at once. Returns the open lock file; dropping it releases.
fn lock(place: &Place) -> Result<std::fs::File, &'static str> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    let name = std::ffi::CString::new("lock").expect("no NUL");
    // SAFETY: a valid dir fd and name; O_NOFOLLOW so a planted link isn't followed.
    let fd = unsafe { libc::openat(place.work.raw(), name.as_ptr(), libc::O_RDWR | libc::O_CREAT | libc::O_NOFOLLOW | libc::O_CLOEXEC, 0o600) };
    if fd < 0 {
        return Err("io");
    }
    let f = std::fs::File::from(unsafe { OwnedFd::from_raw_fd(fd) });
    // SAFETY: a valid fd.
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("setup_running");
    }
    Ok(f)
}

/// What `run` reports to the audit and the caller.
pub struct Done {
    pub report: Report,
    /// A previous copy kept because it had been changed (C5).
    pub backup: Option<String>,
    pub case: &'static str,
}

/// Run a consented install of `prep` (already re-planned by the caller with an equal hash). `ctx.env` is the Setup
/// env; the caller sets HOME to `m.home` and PATH to `ENGINE_PATH` in it (C7).
pub fn run(m: &Machine, ctx: &Ctx, prep: &Prepared, check: &dyn Fn(&Path) -> bool) -> Done {
    let case = match &prep.plan.pin {
        Pin::Engine { case, .. } => *case,
        _ => "fresh",
    };
    let done = |report: Report, backup: Option<String>| {
        audit(ctx, prep, case, &report);
        Done { report, backup, case }
    };
    let fail = |reason: &'static str| done(Report::new(Outcome::Failed, Some(reason), None), None);
    let Ok(place) = Place::open(m.home, m.data, m.uid) else { return fail("not_writable") };
    let _lock = match lock(&place) {
        Ok(l) => l,
        Err(r) => return fail(r),
    };
    if (m.busy)() {
        return fail("engine_in_use");
    }
    // A crash left a journal: recover under this consent first (C6).
    if place.recover().is_err() {
        let _ = ctx.marks.set(ID, &m.home.join(".claude/skills").join(SKILL), None, "rollback_failed");
        return done(Report::new(Outcome::Broken, Some("rollback_failed"), None), None);
    }
    if let Err(r) = state_safe(m.home, m.uid) {
        return fail(r);
    }
    // After recovery the target must be what the consented plan expected (same case and folder identity).
    if place.found() != prep.found {
        return fail("target_changed");
    }
    let Ok(wf) = place.workflows_dir(m.uid) else { return fail("not_writable") };
    let snap = match workflows::snapshot(&wf, prep.want.keys().cloned()) {
        Ok(s) => s,
        Err(e) => return fail(e.code()),
    };
    let sig = sig_state(m.home);
    let staged = match place.stage(&prep.engine, &m.pin.sha256) {
        Ok(s) => s,
        Err(e) => return fail(e.code()),
    };
    let journal = match place.swap_in(&staged, &prep.found) {
        Ok(j) => j,
        Err(e) => {
            let _ = place.work.remove_tree(&staged);
            place.clear_journal();
            return fail(e.code());
        }
    };
    let undo = |reason: &'static str, exit: Option<u32>| {
        let ok = workflows::restore(&wf, &snap).is_ok() & place.roll_back(&journal).is_ok();
        restore_sig(m.home, &sig);
        let restored = match &prep.found {
            Found::Absent => !m.home.join(".claude/skills").join(SKILL).exists(),
            Found::Ours { .. } => place.skills.open_dir(SKILL).is_ok_and(|d| place::unmodified(&d) || matches!(place.found(), Found::Ours { .. })),
            Found::UserCopy => false,
        };
        if ok && restored {
            done(Report::new(Outcome::RolledBack, Some(reason), exit), None)
        } else {
            let _ = ctx.marks.set(ID, &m.home.join(".claude/skills").join(SKILL), None, "rollback_failed");
            done(Report::new(Outcome::Broken, Some("rollback_failed"), exit), None)
        }
    };
    if workflows::write(&wf, &prep.want, &snap).is_err() {
        return undo("io", None);
    }
    let Ok(root) = place.skills.open_dir(SKILL) else { return undo("io", None) };
    if !tree_matches(&root, &prep.engine) {
        return undo("tree_changed", None);
    }
    let cwd = match tempdir() {
        Ok(d) => d,
        Err(_) => return undo("io", None),
    };
    let exit = match spawn_wait(ctx, ID, prep.plan.command.clone(), &cwd) {
        Ok(e) => e,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&cwd);
            return undo(e, None);
        }
    };
    let _ = std::fs::remove_dir_all(&cwd);
    if exit != Some(0) {
        return undo(if exit.is_none() { "install_timeout" } else { "install_failed" }, exit);
    }
    let Ok(root) = place.skills.open_dir(SKILL) else { return undo("io", exit) };
    if !tree_matches(&root, &prep.engine) {
        return undo("tree_changed", exit);
    }
    if !check(&m.home.join(".claude/skills").join(SKILL)) {
        return undo("not_signed", exit);
    }
    if !key_safe(m.home, m.uid) {
        let _ = ctx.marks.set(ID, &m.home.join(".claude/skills").join(SKILL), None, "key_unsafe");
        return done(Report::new(Outcome::Broken, Some("key_unsafe"), exit), None);
    }
    match place.commit(&journal, &prep.engine.version, &m.pin.sha256) {
        Ok(backup) => {
            let _ = ctx.marks.clear(ID);
            done(Report { version: Some(prep.engine.version.clone()), ..Report::new(Outcome::Installed, None, exit) }, backup)
        }
        Err(_) => undo("io", exit),
    }
}

fn tempdir() -> std::io::Result<PathBuf> {
    let mut b = [0u8; 8];
    // SAFETY: fills the buffer.
    unsafe { libc::arc4random_buf(b.as_mut_ptr().cast(), b.len()) };
    let d = std::env::temp_dir().join(format!("vajra-engine-cwd-{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>()));
    std::fs::create_dir(&d)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700))?;
    Ok(d)
}

/// The audit line: case, version, old version, tgz and the result. No paths, tokens or env.
fn audit(ctx: &Ctx, prep: &Prepared, case: &str, r: &Report) {
    let old = match &prep.plan.pin {
        Pin::Engine { old_version, .. } => old_version.clone().unwrap_or_else(|| "none".into()),
        _ => "none".into(),
    };
    let tgz = match &prep.plan.pin {
        Pin::Engine { tgz_sha256, .. } => tgz_sha256.clone(),
        _ => String::new(),
    };
    let exit = r.exit.map_or_else(|| "none".to_string(), |c| c.to_string());
    let line = format!("setup id={ID} method=engine_bundle case={case} version={} old_version={old} tgz={tgz} exit={exit} result={}", prep.engine.version, r.outcome.word());
    let _ = crate::control::audit::append_line(ctx.audit, "SETUP", &line, std::time::SystemTime::now());
}

/// The manifest check, run as the engine's own `--check` (exit 0: every listed file matches and the signature is good).
pub fn session_check(ctx_env: &crate::providers::install::env::SetupEnv) -> impl Fn(&Path) -> bool + '_ {
    move |skill: &Path| {
        let mut cmd = std::process::Command::new("/bin/bash");
        cmd.arg(skill.join(CHECK)).arg("--check").env_clear().current_dir("/").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        for (k, v) in ctx_env.vars() {
            cmd.env(k, v);
        }
        cmd.status().is_ok_and(|s| s.success())
    }
}

#[cfg(test)]
#[path = "flow_tests.rs"]
mod tests;
