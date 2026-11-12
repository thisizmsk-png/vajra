//! What the app knows for proposal seats (ADR-16 16.1, 16.2): the project's agent defaults, and the launch checks.
//! The service holds this as its `SeatPolicy`; start asks it again after overrides.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::control::service::{SeatCheck, SeatPolicy};
use crate::preferences::Agents;
use crate::providers::detect::{self, Out, Probe};
use crate::providers::keychain::Keys;
use crate::providers::launch::{self, Refused};

pub struct SeatEnv {
    data: PathBuf,
    home: PathBuf,
    /// Where provider CLIs are looked for; `None` is the user's resolved PATH. Tests point it at a temp dir.
    dirs: Mutex<Option<Vec<PathBuf>>>,
    keys: Mutex<Arc<Keys>>,
    /// Check 5. This build's value is `launch::CORE_ROUTE_AVAILABLE`; tests turn it on to reach the later checks.
    route: AtomicBool,
    /// 6k: the last launch-time check's answer per Codex account, for the proposal card. Start always checks again.
    locked: Mutex<HashMap<String, crate::adapter::profiles::CodexLock>>,
}

impl SeatEnv {
    pub fn new(data: &Path, home: &Path, keys: Keys) -> Self {
        Self { data: data.to_path_buf(), home: home.to_path_buf(), dirs: Mutex::new(None), keys: Mutex::new(Arc::new(keys)), route: AtomicBool::new(launch::CORE_ROUTE_AVAILABLE), locked: Mutex::new(HashMap::new()) }
    }

    #[cfg(test)]
    pub fn for_test(&self, dirs: Vec<PathBuf>, keys: Keys, route: bool) {
        *crate::lock(&self.dirs) = Some(dirs);
        *crate::lock(&self.keys) = Arc::new(keys);
        self.route.store(route, Ordering::SeqCst);
    }

    pub fn route(&self) -> bool {
        self.route.load(Ordering::SeqCst)
    }

    /// 6k's launch-time check for a Codex seat on `account`, run now: is a session under Vajra's profile kept from the
    /// Vajra state dir, with nothing from that route's config (MCP servers, notify) running outside its sandbox?
    pub fn codex_lock(&self, account: &str) -> crate::adapter::profiles::CodexLock {
        let dirs = crate::lock(&self.dirs).clone().unwrap_or_else(crate::vajra::preflight::path_dirs);
        let system = detect::System::new(dirs).with_home(self.home.clone());
        let login = account == "codex";
        let codex_home = if login {
            // The user's own: `CODEX_HOME` when the app was started with one, else `~/.codex`, as Codex picks it.
            system.env("CODEX_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| self.home.join(".codex"))
        } else {
            match crate::providers::codex_home::home_for(&self.data, account) {
                Ok(h) => h,
                Err(_) => return Err(crate::adapter::profiles::CODEX_NOT_PROVEN.into()),
            }
        };
        let r = crate::adapter::profiles::codex_lock(&system, &self.home, &self.data, &codex_home, login);
        crate::lock(&self.locked).insert(account.to_string(), r.clone());
        r
    }

    /// The last check's answer for `account`, or a fresh check the first time. For the card only: it predicts what
    /// Start will find.
    pub fn codex_lock_hint(&self, account: &str) -> crate::adapter::profiles::CodexLock {
        let last = crate::lock(&self.locked).get(account).cloned();
        last.unwrap_or_else(|| self.codex_lock(account))
    }

    /// The checks of 16.2 for every seat, then the engine once. A project that is not a store id reads the global agents.
    pub fn refusals(&self, project: &str, seats: &[SeatCheck]) -> Vec<(String, Refused)> {
        let Some(agents) = self.agents(project) else {
            return seats.iter().map(|s| (s.seat.clone(), Refused { code: "outside_allowed", detail: "the project's agent choices cannot be read".into() })).collect();
        };
        let dirs = crate::lock(&self.dirs).clone().unwrap_or_else(crate::vajra::preflight::path_dirs);
        let system = detect::System::new(dirs).with_home(self.home.clone());
        // One version and login probe per CLI for the whole check, not one per seat.
        let probe = Memo { inner: &system, runs: Mutex::new(HashMap::new()) };
        let keys = crate::lock(&self.keys).clone();
        let mut out: Vec<(String, Refused)> = seats
            .iter()
            .filter_map(|s| launch::launchable_with(self.route(), &s.choice, s.kind, &agents, &probe, &keys).err().map(|r| (s.seat.clone(), r)))
            .collect();
        if let Err(r) = launch::engine_ready(&self.home) {
            out.push(("engine".into(), r));
        }
        out
    }
}

impl SeatPolicy for SeatEnv {
    fn agents(&self, project: &str) -> Option<Agents> {
        let store = crate::preferences::Store::new(self.data.clone());
        let scope = crate::projects::contract::valid_project_id(project).then_some(project);
        store.read(scope).ok().map(|v| v.agents)
    }
    fn check(&self, project: &str, seats: &[SeatCheck]) -> Vec<(String, Refused)> {
        self.refusals(project, seats)
    }
}

/// One CLI run's answer: exit code, stdout, stderr; `None` when it did not finish.
type Answer = Option<(Option<i32>, String, String)>;

/// Answers a repeated `run` from the first answer.
struct Memo<'a> {
    inner: &'a dyn Probe,
    runs: Mutex<HashMap<(PathBuf, Vec<String>), Answer>>,
}

impl Probe for Memo<'_> {
    fn find(&self, name: &str) -> Option<PathBuf> {
        self.inner.find(name)
    }
    fn run(&self, path: &Path, args: &[&str]) -> Option<Out> {
        let key = (path.to_path_buf(), args.iter().map(|a| a.to_string()).collect::<Vec<_>>());
        let mut m = crate::lock(&self.runs);
        let hit = m.entry(key).or_insert_with(|| self.inner.run(path, args).map(|o| (o.code, o.stdout, o.stderr)));
        hit.clone().map(|(code, stdout, stderr)| Out { code, stdout, stderr })
    }
    fn env(&self, key: &str) -> Option<OsString> {
        self.inner.env(key)
    }
}
