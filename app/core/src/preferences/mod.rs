//! Owner preferences, never execution policy. Read only on demand; callers serialize updates through one Store.
//! Explicit enums, bounded model identifiers and shortcut choices leave no token/env/command-string bucket.
use crate::projects::contract::valid_project_id;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::PathBuf;
const SCHEMA: &str = "vajra.preferences/1";
const MAX_BYTES: usize = 128 * 1024;
const MAX_REV: u64 = (1 << 53) - 1;
macro_rules! choice {($name:ident, $default:ident, $($other:ident),*) => {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all="snake_case")]
    pub enum $name { #[default] $default, $($other),* }
};}
choice!(Theme, System, Dark, Light);
choice!(Editor, System, Code);
choice!(Shell, Login, Sh, Bash, Zsh);
choice!(Agent, Claude, Codex);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Shortcuts {
    pub palette: String,
    pub settings: String,
}
impl Default for Shortcuts {
    fn default() -> Self {
        Self {
            palette: "Primary+K".into(),
            settings: "Primary+,".into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub theme: Theme,
    pub editor: Editor,
    pub shell: Shell,
    pub notifications: bool,
    pub notifications_text: bool,
    pub agent: Agent,
    pub model: String,
    pub workers: u8,
    pub rounds: u8,
    pub shortcuts: Shortcuts,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            editor: Editor::System,
            shell: Shell::Login,
            notifications: true,
            notifications_text: false,
            agent: Agent::Claude,
            model: String::new(),
            workers: 4,
            rounds: 2,
            shortcuts: Shortcuts::default(),
        }
    }
}
/// None means inherit. `inherit` explicitly removes selected project overrides; never a general JSON merge.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Patch {
    pub theme: Option<Theme>,
    pub editor: Option<Editor>,
    pub shell: Option<Shell>,
    pub notifications: Option<bool>,
    pub notifications_text: Option<bool>,
    pub agent: Option<Agent>,
    pub model: Option<String>,
    pub workers: Option<u8>,
    pub rounds: Option<u8>,
    pub shortcuts: Option<Shortcuts>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub inherit: Vec<String>,
}
const FIELDS: [&str; 10] = [
    "theme",
    "editor",
    "shell",
    "notifications",
    "notifications_text",
    "agent",
    "model",
    "workers",
    "rounds",
    "shortcuts",
];
#[derive(Debug, Clone, Copy)]
pub struct Error(&'static str);
impl Error {
    pub fn code(self) -> &'static str {
        self.0
    }
}
fn bad() -> Error {
    Error("invalid_preferences")
}
#[cfg(test)]
thread_local! {
    /// Fault injection: fail the directory sync that follows a successful rename of settings.json.
    pub(crate) static FAIL_SYNC_AFTER_RENAME: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
impl From<std::io::Error> for Error {
    fn from(_: std::io::Error) -> Self {
        Self("preferences_io")
    }
}
impl Preferences {
    fn validate(&self) -> Result<(), Error> {
        if !(1..=16).contains(&self.workers)
            || !(1..=3).contains(&self.rounds)
            || self.model.len() > 80
            || !self
                .model
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._/".contains(&b))
        {
            return Err(bad());
        }
        const KEYS: [&str; 7] = [
            "Primary+K",
            "Primary+,",
            "Primary+Shift+K",
            "Primary+Shift+P",
            "Primary+Shift+S",
            "F6",
            "F7",
        ];
        if !KEYS.contains(&self.shortcuts.palette.as_str())
            || !KEYS.contains(&self.shortcuts.settings.as_str())
            || self.shortcuts.palette == self.shortcuts.settings
        {
            return Err(bad());
        }
        Ok(())
    }
    fn apply(&mut self, p: &Patch) {
        macro_rules! set {($($f:ident),*)=>{$(if let Some(v)=&p.$f{self.$f=*v;})*};}
        set!(
            theme,
            editor,
            shell,
            notifications,
            notifications_text,
            agent,
            workers,
            rounds
        );
        if let Some(v) = &p.model {
            self.model = v.clone();
        }
        if let Some(v) = &p.shortcuts {
            self.shortcuts = v.clone();
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema: String,
    rev: u64,
    global: Preferences,
    overrides: BTreeMap<String, Patch>,
}
impl Default for Document {
    fn default() -> Self {
        Self {
            schema: SCHEMA.into(),
            rev: 0,
            global: Preferences::default(),
            overrides: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct View {
    pub schema: String,
    pub rev: u64,
    pub project_id: Option<String>,
    pub global: Preferences,
    pub overrides: Patch,
    pub effective: Preferences,
    pub sources: BTreeMap<String, String>,
}
impl Document {
    fn view(&self, project: Option<&str>) -> View {
        let patch = project
            .and_then(|id| self.overrides.get(id))
            .cloned()
            .unwrap_or_default();
        let raw = serde_json::to_value(&patch).expect("plain fields");
        let mut effective = self.global.clone();
        effective.apply(&patch);
        View {
            schema: self.schema.clone(),
            rev: self.rev,
            project_id: project.map(str::to_owned),
            global: self.global.clone(),
            overrides: patch,
            effective,
            sources: FIELDS
                .iter()
                .map(|f| {
                    (
                        f.to_string(),
                        if raw[*f].is_null() {
                            "global"
                        } else {
                            "project"
                        }
                        .into(),
                    )
                })
                .collect(),
        }
    }
    fn validate(&self) -> Result<(), Error> {
        if self.schema != SCHEMA {
            return Err(Error("unsupported_version"));
        }
        if self.rev > MAX_REV || self.overrides.len() > 256 {
            return Err(bad());
        }
        self.global.validate()?;
        for (id, p) in &self.overrides {
            if !valid_project_id(id) || !p.inherit.is_empty() {
                return Err(bad());
            }
            let mut effective = self.global.clone();
            effective.apply(p);
            effective.validate()?;
        }
        Ok(())
    }
}
/// No work at construction: no watcher, timer, thread or read until a settings command is invoked.
pub struct Store {
    data: PathBuf,
}
impl Store {
    pub fn new(data: PathBuf) -> Self {
        Self { data }
    }
    fn scope(project: Option<&str>) -> Result<(), Error> {
        if project.is_some_and(|id| !valid_project_id(id)) {
            return Err(bad());
        }
        Ok(())
    }
    pub fn read(&self, project: Option<&str>) -> Result<View, Error> {
        Self::scope(project)?;
        let dir = self.directory()?;
        lock_file(&dir, false)?;
        Ok(Self::load(&dir)?.view(project))
    }
    pub fn update(
        &self,
        project: Option<&str>,
        expect_rev: u64,
        patch: Patch,
        reset: bool,
    ) -> Result<View, Error> {
        Self::scope(project)?;
        let dir = self.directory()?;
        lock_file(&dir, true)?;
        let mut doc = Self::load(&dir)?;
        if doc.rev != expect_rev {
            return Err(Error("revision_conflict"));
        }
        let raw = serde_json::to_value(&patch).map_err(|_| bad())?;
        if reset && (FIELDS.iter().any(|f| !raw[*f].is_null()) || !patch.inherit.is_empty()) {
            return Err(bad());
        }
        if let Some(id) = project {
            if reset {
                doc.overrides.remove(id);
            } else {
                let entry = doc.overrides.entry(id.into()).or_default();
                let mut merged = serde_json::to_value(&*entry).map_err(|_| bad())?;
                for field in &patch.inherit {
                    if !FIELDS.contains(&field.as_str()) || !raw[field].is_null() {
                        return Err(bad());
                    }
                    merged[field] = Value::Null;
                }
                for field in FIELDS {
                    if !raw[field].is_null() {
                        merged[field] = raw[field].clone();
                    }
                }
                *entry = serde_json::from_value(merged).map_err(|_| bad())?;
            }
        } else {
            if !patch.inherit.is_empty() {
                return Err(bad());
            }
            if reset {
                doc.global = Preferences::default();
            } else {
                doc.global.apply(&patch);
            }
        }
        doc.rev = doc
            .rev
            .checked_add(1)
            .filter(|r| *r <= MAX_REV)
            .ok_or_else(bad)?;
        doc.validate()?;
        let bytes = serde_json::to_vec(&doc).map_err(|_| bad())?;
        if bytes.len() > MAX_BYTES {
            return Err(bad());
        }
        Self::write(&dir, &bytes)?;
        Ok(doc.view(project))
    }
    /// Anchor all child operations to held descriptors; project ids are keys, never filesystem paths.
    fn directory(&self) -> Result<File, Error> {
        let base = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(&self.data)?;
        // SAFETY: fixed NUL-terminated relative name, valid held directory fd.
        let made = unsafe { libc::mkdirat(base.as_raw_fd(), c"preferences".as_ptr(), 0o700) };
        if made != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
            return Err(std::io::Error::last_os_error().into());
        }
        let dir = open_at(
            &base,
            c"preferences",
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW,
            0,
        )?;
        if dir.metadata()?.uid() != unsafe { libc::geteuid() } {
            return Err(bad());
        }
        // SAFETY: owned directory descriptor; never a path that can be replaced with a link.
        if unsafe { libc::fchmod(dir.as_raw_fd(), 0o700) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(dir)
    }
    fn load(dir: &File) -> Result<Document, Error> {
        let file = match open_at(
            dir,
            c"settings.json",
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            0,
        ) {
            Ok(f) => f,
            Err(e) if e.raw_os_error() == Some(libc::ENOENT) => return Ok(Document::default()),
            Err(e) => return Err(e.into()),
        };
        let meta = file.metadata()?;
        if !meta.is_file()
            || meta.len() > MAX_BYTES as u64
            || meta.uid() != unsafe { libc::geteuid() }
        {
            return Err(bad());
        }
        let mut bytes = Vec::new();
        file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > MAX_BYTES {
            return Err(bad());
        }
        let raw: Value = serde_json::from_slice(&bytes).map_err(|_| bad())?;
        if raw["schema"].as_str() != Some(SCHEMA) {
            return Err(Error("unsupported_version"));
        }
        let doc: Document = serde_json::from_value(raw).map_err(|_| bad())?;
        doc.validate()?;
        Ok(doc)
    }
    fn write(dir: &File, bytes: &[u8]) -> Result<(), Error> {
        let name = std::ffi::CString::new(format!(
            ".settings-{}.tmp",
            crate::ids::uuid_v4().map_err(Error::from)?
        ))
        .map_err(|_| bad())?;
        let staged = (|| {
            let mut f = open_at(
                dir,
                &name,
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW,
                0o600,
            )?;
            f.write_all(bytes)?;
            f.sync_all()?;
            // SAFETY: both paths are fixed/generated relative names and both directory fds remain held.
            if unsafe {
                libc::renameat(
                    dir.as_raw_fd(),
                    name.as_ptr(),
                    dir.as_raw_fd(),
                    c"settings.json".as_ptr(),
                )
            } != 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(())
        })();
        if staged.is_err() {
            // Nothing was replaced: drop the temporary file and report the refusal.
            unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), 0) };
            return staged;
        }
        // From here settings.json already holds the new document and revision. A failure now is not a refusal:
        // the update may well have happened, so it is reported as an unknown outcome to reload and review.
        #[cfg(test)]
        if FAIL_SYNC_AFTER_RENAME.with(|f| f.get()) {
            return Err(Error("outcome_unknown"));
        }
        dir.sync_all().map_err(|_| Error("outcome_unknown"))
    }
}
fn lock_file(dir: &File, exclusive: bool) -> Result<(), Error> {
    // SAFETY: descriptor stays alive until the operation finishes; LOCK_NB prevents an unbounded UI wait.
    if unsafe {
        libc::flock(
            dir.as_raw_fd(),
            (if exclusive {
                libc::LOCK_EX
            } else {
                libc::LOCK_SH
            }) | libc::LOCK_NB,
        )
    } == 0
    {
        Ok(())
    } else {
        Err(Error("preferences_busy"))
    }
}
fn open_at(
    dir: &File,
    name: &std::ffi::CStr,
    flags: i32,
    mode: libc::mode_t,
) -> std::io::Result<File> {
    // SAFETY: fd and C string live for this call; successful returned fd is owned exactly once by File.
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CLOEXEC,
            mode as libc::c_uint,
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}
impl View {
    pub fn json(self) -> Value {
        let mut value = json!(self);
        value["caps"] = json!({"active_runs":1,"workers_per_run":16,"editable":false});
        value
    }
}
#[cfg(test)]
mod tests;
