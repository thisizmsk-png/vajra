//! H8 launch checks (ADR-16 16.2): can this seat's choice be run as a worker, right now? Run at proposal and again at
//! Start (after overrides). It checks only what 16.2 lists, and never reads a key (`Keys::saved` is an existence test).
//! Login state is advisory and model ids are opaque: neither is checked here.

use std::path::Path;

use super::contract::Tested;
use super::detect::{self, Probe};
use super::install::marks::Marks;
use super::keychain::{Keys, KEY_ACCOUNTS};
use crate::adapter::Engine;
use crate::crew::roles::Kind;
use crate::preferences::{Agents, Choice};

/// Check 5. 6d flips this when the run protocol ships; until then a proposal with seats cannot start.
pub const CORE_ROUTE_AVAILABLE: bool = false;

/// Check 4's table: the role kinds each harness has an adapter and a profile for (N5). Claude Code and Codex run all.
const SUPPORTED: [(Engine, &[Kind]); 2] = [(Engine::Claude, &[Kind::Lead, Kind::Writer, Kind::Checker]), (Engine::Codex, &[Kind::Lead, Kind::Writer, Kind::Checker])];

/// The engine's tested range (check 6): its `package.json` version, as (major, minor).
const ENGINE_MIN: (u32, u32) = (0, 6);
const ENGINE_MAX: (u32, u32) = (0, 999);
pub const ENGINE_INSTALL: &str = "Set it up in Settings > Providers > Set up.";
const ENGINE_FILES: [&str; 4] = ["scripts/vajra-campaign.sh", "scripts/vajra-verify-work.sh", "scripts/vajra_tasks.py", "scripts/vajra_kb.py"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    pub code: &'static str,
    pub detail: String,
}

fn refuse(code: &'static str, detail: impl Into<String>) -> Result<(), Refused> {
    Err(Refused { code, detail: detail.into() })
}

/// The harness an account runs on, and whether it needs a stored key. A gateway account runs on Codex (`CODEX_HOME`).
fn route_of(account: &str) -> Option<(Engine, bool)> {
    match account {
        "claude" => Some((Engine::Claude, false)),
        "codex" => Some((Engine::Codex, false)),
        a if KEY_ACCOUNTS.contains(&a) => Some((Engine::Codex, true)),
        _ => None,
    }
}

/// The harness an account runs on (`None`: no adapter).
pub fn harness_of(account: &str) -> Option<Engine> {
    route_of(account).map(|(e, _)| e)
}

/// Checks 1 to 5, in order; the first failure is the seat's refusal.
pub fn launchable(choice: &Choice, kind: Kind, agents: &Agents, probe: &dyn Probe, keys: &Keys) -> Result<(), Refused> {
    launchable_with(CORE_ROUTE_AVAILABLE, choice, kind, agents, probe, keys)
}

pub fn launchable_with(route_available: bool, choice: &Choice, kind: Kind, agents: &Agents, probe: &dyn Probe, keys: &Keys) -> Result<(), Refused> {
    launchable_marked(route_available, choice, kind, agents, probe, keys, &Marks::none())
}

/// `launchable_with`, also refusing a harness a failed one-click install marked Broken (bhishma condition 1).
pub fn launchable_marked(route_available: bool, choice: &Choice, kind: Kind, agents: &Agents, probe: &dyn Probe, keys: &Keys, marks: &Marks) -> Result<(), Refused> {
    if !agents.permits(choice) {
        return refuse("outside_allowed", "this choice is outside the project's allowed list");
    }
    let Some((engine, needs_key)) = route_of(&choice.account) else {
        return refuse("harness_unsupported", format!("{} can be set up, but Vajra can't run it as a worker yet.", choice.account));
    };
    let Some(info) = detect::one(probe, engine.id()) else { return refuse("harness_missing", "the harness is not known to this build") };
    if !info.installed {
        return refuse("harness_missing", format!("{} is not installed. Settings > Providers > Set up.", info.name));
    }
    if info.version.is_none() {
        return refuse("harness_broken", format!("{} did not report a version. Settings > Providers > Set up.", info.name));
    }
    if let Some(path) = info.path.as_deref() {
        if marks.active(&info.id, Path::new(path), info.version.as_deref()).is_some() {
            return refuse("harness_broken", format!("{}'s last install did not check out. Settings > Providers > Set up.", info.name));
        }
    }
    if needs_key {
        match keys.saved(&choice.account) {
            Ok(true) => {}
            Ok(false) => return refuse("key_missing", format!("no key is saved for {}", choice.account)),
            Err(_) => return refuse("key_unreadable", format!("the Keychain did not answer for {}", choice.account)),
        }
    }
    let version = info.version.as_deref().unwrap_or("");
    let kinds = SUPPORTED.iter().find(|(e, _)| *e == engine).map(|(_, k)| *k).unwrap_or(&[]);
    if !kinds.contains(&kind) {
        return refuse("harness_unsupported", format!("{} can be set up, but Vajra can't run it for this role yet.", info.name));
    }
    if info.tested != Tested::Tested {
        let range = info.tested_range.as_deref().unwrap_or("?");
        return refuse("harness_unsupported", format!("{} {version} hasn't been tested with Vajra yet (tested: {range}).", info.name));
    }
    if !route_available {
        return refuse("core_route_unavailable", "this build can't run a crew of worker picks yet");
    }
    Ok(())
}

/// Check 6, once per proposal: the engine scripts are installed under `home`, at a version the core was tested with,
/// not marked Broken by a failed engine install, and not being installed right now (D7, bhishma C8, C9). The engine's
/// mark is only read here, never cleared: it ends only with a passing engine install.
pub fn engine_ready(home: &Path, marks: &crate::providers::install::marks::Marks) -> Result<(), Refused> {
    use crate::providers::install::engine::flow;
    if flow::installing(home) {
        return refuse("engine_installing", "the engine is being installed. Start again when Set up finishes.");
    }
    if let Some(reason) = marks.any(flow::ID) {
        return refuse("engine_broken", format!("the last engine install could not be undone cleanly ({reason}). {ENGINE_INSTALL}"));
    }
    let root = home.join(".claude/skills/vajra");
    if let Some(f) = ENGINE_FILES.iter().find(|f| !root.join(f).is_file()) {
        return refuse("engine_missing", format!("the engine script {f} is not installed. {ENGINE_INSTALL}"));
    }
    let version = std::fs::read(root.join("package.json")).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()).and_then(|v| v.get("version")?.as_str().map(str::to_string));
    let mm = version.as_deref().and_then(|v| {
        let mut p = v.split('.').map(|n| n.parse::<u32>().ok());
        Some((p.next()??, p.next()??))
    });
    match mm {
        Some(m) if (ENGINE_MIN..=ENGINE_MAX).contains(&m) => Ok(()),
        _ => refuse("engine_unsupported", format!("the installed engine ({}) is outside the tested range {}.{}-{}.x. {ENGINE_INSTALL}", version.as_deref().unwrap_or("unknown"), ENGINE_MIN.0, ENGINE_MIN.1, ENGINE_MAX.0)),
    }
}

#[cfg(test)]
#[path = "launch_tests.rs"]
mod tests;
