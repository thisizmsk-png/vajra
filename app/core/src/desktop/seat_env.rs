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
    labs: super::labs::Labs,
}

impl SeatEnv {
    pub fn new(data: &Path, home: &Path, keys: Keys) -> Self {
        Self { data: data.to_path_buf(), home: home.to_path_buf(), dirs: Mutex::new(None), keys: Mutex::new(Arc::new(keys)), route: AtomicBool::new(launch::CORE_ROUTE_AVAILABLE), labs: super::labs::Labs::new(data) }
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

    /// Check 5 for one project: the build's core route, or Labs on for it (developer builds only).
    pub fn route_for(&self, project: &str) -> bool {
        self.route() || self.labs.on(project)
    }

    pub fn labs(&self) -> &super::labs::Labs {
        &self.labs
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
            .filter_map(|s| launch::launchable_with(self.route_for(project), &s.choice, s.kind, &agents, &probe, &keys).err().map(|r| (s.seat.clone(), r)))
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
    fn seats_on(&self, project: &str) -> bool {
        self.route_for(project)
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
