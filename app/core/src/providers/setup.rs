//! Settings > Providers > Set up (ADR-17, step 1 of N12): which catalog harnesses are on this machine, the vendor's own
//! install steps for the ones that are not, and the CLI's own sign-in.
//!
//! This step installs nothing. A missing or broken harness shows the vendor's commands and docs link for the user to run
//! themselves; there is no consent, plan or spawn of an installer here. Detection is read-only: the binary on the user's
//! PATH and its `--version` (the same `Probe` and timeout as `providers::detect`); no output is stored.
//!
//! Sign-in runs the catalog's `sign_in.argv` on the detected binary in a user terminal (`terminal::Terminals::
//! open_sign_in`), never on the bus or an agent surface. Vajra never sees the token: the CLI writes its own login.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::catalog::{self, Harness};
use super::detect::{is_tested, version_of, Probe};
use super::install::marks::Marks;

pub const SETUP_SCHEMA: &str = "vajra.harness_setup/1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// On the PATH, reports a version, and Vajra has an adapter for it.
    Installed,
    /// Installed, but Vajra can't run it as an agent yet (ADR-16 N5).
    Unsupported,
    /// On the PATH but its version check fails.
    Broken,
    /// Not installed: the vendor's steps, for the user to run.
    Manual,
}

#[derive(Debug, Clone, Serialize)]
pub struct Command {
    /// `brew`, `npm` or `script`.
    pub tool: String,
    pub command: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetupRow {
    pub id: String,
    pub name: String,
    pub state: State,
    pub version: Option<String>,
    pub path: Option<String>,
    /// Why a row is `manual` or `broken`, for the UI's own wording.
    pub reason_code: Option<String>,
    /// The vendor's commands, in catalog order; empty when it is installed.
    pub commands: Vec<Command>,
    pub docs_url: String,
    pub can_sign_in: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineRow {
    pub installed: bool,
    pub command: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetupList {
    pub schema: String,
    pub rows: Vec<SetupRow>,
    pub engine: EngineRow,
}

/// What the version check found for an entry.
enum Found {
    Missing,
    Broken(PathBuf),
    Ok(PathBuf, String, (u32, u32)),
}

fn found(h: &Harness, p: &dyn Probe) -> Found {
    let Some(path) = p.find(&h.binary) else { return Found::Missing };
    let args: Vec<&str> = h.detect_args.iter().map(String::as_str).collect();
    match p.run(&path, &args).filter(|o| o.code == Some(0)).and_then(|o| version_of(&o.stdout)) {
        Some((v, mm)) => Found::Ok(path, v, mm),
        None => Found::Broken(path),
    }
}

fn commands(h: &Harness) -> Vec<Command> {
    h.methods.iter().map(|m| Command { tool: m.tool().unwrap_or("script").into(), command: m.command().into() }).collect()
}

fn row(h: &Harness, p: &dyn Probe, marks: &Marks) -> SetupRow {
    let base = |state, version: Option<String>, path: Option<&Path>, reason: Option<&str>, commands: Vec<Command>| SetupRow {
        id: h.id.clone(),
        name: h.name.clone(),
        state,
        version,
        path: path.map(|p| p.to_string_lossy().into_owned()),
        reason_code: reason.map(str::to_string),
        commands,
        docs_url: h.docs_url.clone(),
        can_sign_in: matches!(state, State::Installed | State::Unsupported),
    };
    match found(h, p) {
        // Every entry is "Install it yourself" in this step; an unverified source would be so even with one-click.
        Found::Missing => base(State::Manual, None, None, Some("source_not_verified"), commands(h)),
        Found::Broken(path) => base(State::Broken, None, Some(&path), Some("version_failed"), commands(h)),
        Found::Ok(path, v, mm) => {
            // A failed closure check or rollback holds until a passing check or a different binary or version (condition 1).
            if let Some(reason) = marks.active(&h.id, &path, Some(&v)) {
                return base(State::Broken, Some(v), Some(&path), Some(&reason), commands(h));
            }
            // Runnable only with an adapter tested on this version (ADR-17 rule 9), the same range launch checks.
            let state = if h.adapter.as_deref().is_some_and(|a| is_tested(a, mm)) { State::Installed } else { State::Unsupported };
            base(state, Some(v), Some(&path), None, Vec::new())
        }
    }
}

/// One row per catalog entry, plus the engine-scripts row (`engine_installed` from the caller's preflight read). Always
/// answers; a catalog that does not parse gives no rows.
pub fn list(p: &dyn Probe, engine_installed: bool) -> SetupList {
    list_marked(p, engine_installed, &Marks::none())
}

/// `list`, with the Broken marks of a failed one-click install (`install::marks`).
pub fn list_marked(p: &dyn Probe, engine_installed: bool, marks: &Marks) -> SetupList {
    let cat = catalog::catalog().ok();
    SetupList {
        schema: SETUP_SCHEMA.into(),
        rows: cat.map(|c| c.harnesses.iter().map(|h| row(h, p, marks)).collect()).unwrap_or_default(),
        engine: EngineRow { installed: engine_installed, command: cat.map(|c| c.engine_scripts.manual_command.clone()).unwrap_or_default() },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupError {
    Unknown,
    NotInstalled,
    Io(String),
}

impl SetupError {
    pub fn code(&self) -> &'static str {
        match self {
            SetupError::Unknown => "harness_unknown",
            SetupError::NotInstalled => "harness_missing",
            SetupError::Io(_) => "io",
        }
    }
}

impl std::fmt::Display for SetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SetupError::Unknown => f.write_str("that harness is not in the catalog"),
            SetupError::NotInstalled => f.write_str("that harness is not installed or does not report a version"),
            SetupError::Io(e) => write!(f, "could not prepare the sign-in folder: {e}"),
        }
    }
}

/// A CLI's own sign-in, ready for a user terminal. Only `sign_in` builds one, from the compiled catalog and the binary
/// detection found (the absolute PATH entry it ran, which must resolve to a real file), so a terminal can be asked to run
/// nothing else.
#[derive(Debug)]
pub struct SignInCommand {
    id: String,
    name: String,
    program: PathBuf,
    args: Vec<String>,
    cwd: PathBuf,
}

impl SignInCommand {
    /// The catalog harness id.
    pub fn id(&self) -> &str {
        &self.id
    }
    /// The harness's display name.
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn program(&self) -> &Path {
        &self.program
    }
    pub fn args(&self) -> &[String] {
        &self.args
    }
    /// A fresh, empty, private (0700) folder: never HOME or a repo, so no project config applies.
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }
}

/// `mkdtemp` under the temp dir: a new 0700 folder no other sign-in shares.
pub(crate) fn private_dir(tag: &str) -> Result<PathBuf, SetupError> {
    use std::os::unix::ffi::OsStringExt;
    let tmpl = std::env::temp_dir().join(format!("{tag}-XXXXXXXX"));
    let mut bytes = std::ffi::CString::new(tmpl.into_os_string().into_vec()).map_err(|e| SetupError::Io(e.to_string()))?.into_bytes_with_nul();
    // SAFETY: a writable NUL-terminated template that outlives the call.
    let p = unsafe { libc::mkdtemp(bytes.as_mut_ptr().cast()) };
    if p.is_null() {
        return Err(SetupError::Io(std::io::Error::last_os_error().to_string()));
    }
    bytes.pop();
    let dir = PathBuf::from(std::ffi::OsString::from_vec(bytes));
    dir.canonicalize().map_err(|e| SetupError::Io(e.to_string()))
}

/// The sign-in for `id`: refused unless the harness is installed and reports a version.
pub fn sign_in(id: &str, p: &dyn Probe) -> Result<SignInCommand, SetupError> {
    let h = catalog::find(id).ok_or(SetupError::Unknown)?;
    let Found::Ok(path, _, _) = found(h, p) else { return Err(SetupError::NotInstalled) };
    // Run the PATH entry detection ran, by that name: a version-manager shim is a symlink to one binary that dispatches
    // on argv[0], so its symlink target would run the manager, not the CLI. It must still resolve to a real file.
    let real = path.canonicalize().map_err(|_| SetupError::NotInstalled)?;
    if !path.is_absolute() || !real.is_file() {
        return Err(SetupError::NotInstalled);
    }
    Ok(SignInCommand { id: h.id.clone(), name: h.name.clone(), program: path, args: h.sign_in.argv[1..].to_vec(), cwd: private_dir("vajra-signin")? })
}

#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;
