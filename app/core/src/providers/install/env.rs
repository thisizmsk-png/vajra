//! The Setup env profile (ADR-17 rule 5, bhishma condition 8): its own branch, not the agent `extra_env`. The allowed
//! names (`sessions::ENV_ALLOW`), PATH set to the resolved tool folders plus the fixed base PATH (never the user's
//! PATH), and only the proxy and CA variables the app itself has. No key, no token, no npm or Homebrew override.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::crew::redact::Redactor;

/// Every name a Setup run may see.
pub const ALLOWED: [&str; 9] = ["PATH", "HOME", "LANG", "TERM", "USER", "SHELL", "HTTPS_PROXY", "NO_PROXY", "NODE_EXTRA_CA_CERTS"];
const PROXY: [&str; 3] = ["HTTPS_PROXY", "NO_PROXY", "NODE_EXTRA_CA_CERTS"];

/// The env for setup helpers and the install itself. Only `setup_env` builds one.
#[derive(Debug, Clone)]
pub struct SetupEnv(Vec<(&'static str, OsString)>);

impl SetupEnv {
    pub fn vars(&self) -> &[(&'static str, OsString)] {
        &self.0
    }
    fn get(&self, k: &str) -> Option<&OsString> {
        self.0.iter().find(|(n, _)| *n == k).map(|(_, v)| v)
    }
}

fn usable(v: &OsString) -> bool {
    use std::os::unix::ffi::OsStrExt;
    !v.is_empty() && !v.as_bytes().contains(&0)
}

/// Build the Setup env from `tool_dirs` (absolute only) and the app's own variables (`get`).
pub fn setup_env(tool_dirs: &[PathBuf], get: impl Fn(&str) -> Option<OsString>) -> SetupEnv {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for d in tool_dirs.iter().cloned().chain(crate::vajra::bridge::BASE_PATH.map(PathBuf::from)) {
        if d.is_absolute() && !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    let path = std::env::join_paths(&dirs).unwrap_or_default();
    let mut vars: Vec<(&'static str, OsString)> = crate::sessions::scrubbed_env(|k| if k == "PATH" { Some(path.clone()) } else { get(k).filter(usable) });
    for k in PROXY {
        if let Some(v) = get(k).filter(usable) {
            vars.push((k, v));
        }
    }
    SetupEnv(vars)
}

/// The redactor for a `setup-<id>` stream: the proxy value and its credentials are replaced wherever they appear.
pub fn redactor(env: &SetupEnv) -> Redactor {
    let mut r = Redactor::default();
    if let Some(v) = env.get("HTTPS_PROXY").map(|v| v.to_string_lossy().into_owned()) {
        let authority = v.split_once("://").map_or(v.as_str(), |(_, rest)| rest).split('/').next().unwrap_or("");
        if let Some((user, host_port)) = authority.rsplit_once('@') {
            let host = host_port.rsplit_once(':').map_or(host_port, |(h, _)| h);
            for secret in [v.clone(), authority.to_string(), format!("{user}@{host}")] {
                r.add(Some(&secret));
            }
        }
    }
    r
}

#[cfg(test)]
#[path = "env_tests.rs"]
mod tests;
