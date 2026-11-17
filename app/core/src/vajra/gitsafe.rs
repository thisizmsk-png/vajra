//! Read-only git on a folder whose config and attributes an agent may have written (bhishma L1, 17 Nov).
//!
//! Git runs programs that the repository names: a `clean` or `process` filter (an attribute plus a `filter.<driver>`
//! entry) whenever it reads a working file to compare it, a `textconv` or external diff driver, `core.fsmonitor`, hooks,
//! `gpg.program` for a signature shown by `log`, and a transport (`core.sshCommand`) for a lazy fetch in a partial
//! clone. The config and `info/attributes` live in the repo's git dir and the `.gitattributes` files in its worktree,
//! all writable by an agent, so the desktop's own `git status` or `git diff` could run an agent's command outside its
//! sandbox. Git has no option that skips filters on a working-tree compare, and reading the driver names first and
//! then overriding each with `-c` races an agent that rewrites the config in between.
//!
//! So every bridged git call that can read a working file, a diff or a signature (`needs_isolation`: all but
//! `--version`, `rev-parse` and `merge-base`) runs `isolate`d: the repository's own config files are never read by it.
//! - One `rev-parse` finds the git dir, common dir and top level, and one `config --list` reads the effective config
//!   (system file off, as on every bridged call). Neither reads a working file, an object or an index, so neither runs
//!   a filter, driver, hook or fetch.
//! - From that one read, only `PASSED` keys go on, as command-scope `GIT_CONFIG_KEY_<n>` pairs: data the output
//!   depends on (case, unicode and mode handling, line endings, excludes, a branch's upstream). Never a key naming a
//!   program, so no filter, driver, hook, signer or transport is defined at all; an attribute naming one does nothing.
//! - The command then runs with `GIT_COMMON_DIR` set to a private dir (mode 0700, removed afterwards) whose `config`
//!   holds only the repository format, and whose `refs`, `packed-refs`, `objects`, `info`, `logs`, `shallow` and
//!   `reftable` are links to the real ones. `GIT_DIR` is the real git dir (`HEAD`, the index) and `GIT_WORK_TREE` the
//!   real top level. Git reads the local config from the common dir, and `config.worktree` only when that config
//!   sets `extensions.worktreeConfig`, which it never does; `GIT_CONFIG_GLOBAL=/dev/null` and `GIT_CONFIG_NOSYSTEM=1`
//!   leave out the other files. A config written after the one read is never seen.
//! - Every call also gets `HARDENING` (`-c`, outranking any file) and `ENV`: no optional index write, no lazy fetch,
//!   no system attributes file.
//!
//! What git then no longer does here, on purpose: run any filter (git LFS's included, so an LFS file whose stat
//! changed reads as modified, and a diff shows its working bytes against the pointer), a textconv or external diff,
//! fetch a partial clone's missing object (the call fails instead), or honour other repo config (diff, status and log
//! options are git's defaults).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use super::bridge::{run_process_env, Output};

/// `-c` on every bridged git call: no fsmonitor, hook, transport, credential helper or background maintenance.
pub const HARDENING: [&str; 8] = [
    "core.fsmonitor=",
    "core.hooksPath=/dev/null",
    "core.sshCommand=false",
    "protocol.allow=never",
    "credential.helper=",
    "submodule.recurse=false",
    "gc.auto=0",
    "maintenance.auto=false",
];

/// Environment on every bridged git call, on top of the bridge's scrubbed one (which sets `GIT_CONFIG_NOSYSTEM=1`).
pub const ENV: [(&str, &str); 3] = [("GIT_OPTIONAL_LOCKS", "0"), ("GIT_NO_LAZY_FETCH", "1"), ("GIT_ATTR_NOSYSTEM", "1")];

/// The `core.*` keys an isolated call keeps: data git reads, never a program or a hook.
const CORE_PASSED: [&str; 16] = [
    "filemode",
    "ignorecase",
    "precomposeunicode",
    "symlinks",
    "autocrlf",
    "eol",
    "safecrlf",
    "checkstat",
    "trustctime",
    "quotepath",
    "excludesfile",
    "sparsecheckout",
    "sparsecheckoutcone",
    "bigfilethreshold",
    "abbrev",
    "checkroundtripencoding",
];

/// The common-dir entries an isolated call links to: what read commands look up there besides `config` and `hooks`.
const LINKED: [&str; 7] = ["refs", "packed-refs", "objects", "info", "logs", "shallow", "reftable"];

/// Whether a bridged subcommand runs `isolate`d. `--version`, `rev-parse` and `merge-base` read only refs, config and
/// commit objects: never a working file, an attribute, a diff or a signature, so no filter, driver or signer applies,
/// and `HARDENING` covers the rest (fsmonitor, hooks, transports). Everything else (`status`, `diff`, `log`,
/// `ls-files`, and any subcommand added later) is isolated.
pub fn needs_isolation(sub: &str) -> bool {
    !matches!(sub, "--version" | "rev-parse" | "merge-base")
}

/// `--no-pager` and `HARDENING` as `-c` pairs: the start of every bridged git argv.
pub fn prefix() -> Vec<String> {
    let mut v = vec!["--no-pager".to_string()];
    for kv in HARDENING {
        v.extend(["-c".to_string(), kv.to_string()]);
    }
    v
}

/// Whether one key from `config --list` goes on to an isolated call: `PASSED` in the module doc.
fn passed(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    let Some((section, rest)) = lower.split_once('.') else { return false };
    let last = rest.rsplit('.').next().unwrap_or("");
    let sub = rest.len() > last.len(); // has a subsection
    match section {
        "core" => !sub && CORE_PASSED.contains(&rest),
        // `@{u}`: the branch's remote and merge ref, and the remote's fetch refspecs that map it to a tracking ref.
        "branch" => sub && matches!(last, "remote" | "merge"),
        "remote" => sub && last == "fetch",
        _ => false,
    }
}

/// The repository format a private common dir's config must name, from the effective config.
#[derive(Debug, Default, PartialEq)]
struct Format {
    extensions: Vec<(&'static str, String)>,
}

/// Parse `config --list -z` (`key LF value NUL`, or `key NUL` for a bare boolean) into the passed pairs and the
/// format. `None` on bad output or a format value this code does not know (git would refuse that repo too).
fn parse(out: &[u8]) -> Option<(Vec<(String, String)>, Format)> {
    let text = std::str::from_utf8(out).ok()?;
    let mut pieces: Vec<&str> = text.split('\0').collect();
    if pieces.pop() != Some("") {
        return None; // every record ends in NUL
    }
    let (mut pairs, mut fmt) = (Vec::new(), Format::default());
    for item in pieces {
        let (key, value) = item.split_once('\n').unwrap_or((item, "true"));
        if key.is_empty() {
            return None;
        }
        let lower = key.to_ascii_lowercase();
        let ext = match lower.as_str() {
            "extensions.objectformat" => Some(("objectformat", &["sha1", "sha256"][..])),
            "extensions.compatobjectformat" => Some(("compatobjectformat", &["sha1", "sha256"][..])),
            "extensions.refstorage" => Some(("refstorage", &["files", "reftable"][..])),
            _ => None,
        };
        if let Some((name, allowed)) = ext {
            let v = value.to_ascii_lowercase();
            if !allowed.contains(&v.as_str()) {
                return None;
            }
            fmt.extensions.retain(|(n, _)| *n != name);
            fmt.extensions.push((name, v));
        } else if passed(key) {
            pairs.push((key.to_string(), value.to_string()));
        }
    }
    Some((pairs, fmt))
}

/// The private common dir's `config`: the repository format alone.
fn format_config(fmt: &Format) -> String {
    if fmt.extensions.is_empty() {
        return "[core]\n\trepositoryformatversion = 0\n".into();
    }
    let mut s = "[core]\n\trepositoryformatversion = 1\n[extensions]\n".to_string();
    for (k, v) in &fmt.extensions {
        s.push_str(&format!("\t{k} = {v}\n"));
    }
    s
}

/// A private common dir and the environment that points git at it. The dir is removed when this is dropped.
#[derive(Debug)]
pub struct Isolated {
    dir: PathBuf,
    env: Vec<(String, OsString)>,
}

impl Isolated {
    /// The environment for the isolated call (with `ENV`), for `run_process_env`.
    pub fn env(&self) -> Vec<(&str, &Path)> {
        self.env.iter().map(|(k, v)| (k.as_str(), Path::new(v))).collect()
    }
}

impl Drop for Isolated {
    fn drop(&mut self) {
        // `remove_dir_all` removes the links themselves, never what they point at.
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The environment pairs every bridged git call gets (`ENV`).
pub fn base_env() -> Vec<(&'static str, &'static Path)> {
    ENV.iter().map(|(k, v)| (*k, Path::new(*v))).collect()
}

/// A git run that did not happen, as the caller reads one: exit 128 (git's own for "cannot run here"), or timed out.
fn not_run(why: &str, timed_out: bool) -> Output {
    Output { code: if timed_out { None } else { Some(128) }, stdout: String::new(), stdout_raw: Vec::new(), stderr: format!("vajra: {why}\n"), truncated: false, timed_out }
}

/// Set up an isolated call for `cwd` (module doc), within `deadline`. `Err` holds what the caller should see instead
/// of the command's output: the failing step's own output (`cwd` is not in a work tree: git's exit 128 and message),
/// or `not_run`.
pub fn isolate(git: &Path, cwd: &Path, home: Option<&Path>, deadline: Instant) -> Result<Isolated, Output> {
    let step = |args: &[&str]| -> Result<Output, Output> {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(not_run("git ran out of time", true));
        }
        let argv: Vec<String> = prefix().into_iter().chain(args.iter().map(|a| a.to_string())).collect();
        let o = run_process_env(git, &argv, Some(cwd), left, home, None, &base_env()).map_err(|e| not_run(&e.to_string(), false))?;
        if o.exit() == Some(0) { Ok(o) } else { Err(o) }
    };
    let found = step(&["rev-parse", "--path-format=absolute", "--absolute-git-dir", "--git-common-dir", "--show-toplevel"])?;
    let lines: Vec<&str> = found.stdout.strip_suffix('\n').unwrap_or(&found.stdout).split('\n').collect();
    let dirs: Vec<PathBuf> = lines.iter().filter_map(|l| Path::new(l).is_absolute().then(|| Path::new(l).canonicalize().ok()).flatten()).filter(|p| p.is_dir()).collect();
    let [gitdir, common, top] = <[PathBuf; 3]>::try_from(dirs).map_err(|_| not_run("git did not name a work tree", false))?;
    let objects = common.join("objects").canonicalize().ok().filter(|p| p.is_dir()).ok_or_else(|| not_run("the repository has no object store", false))?;
    let listed = step(&["config", "--list", "-z"])?;
    if listed.truncated {
        return Err(not_run("the repository config is too large", false));
    }
    let (pairs, fmt) = parse(&listed.stdout_raw).ok_or_else(|| not_run("the repository config could not be read", false))?;

    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("vajra-git-{}-{}-{nanos}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
    {
        use std::os::unix::fs::DirBuilderExt;
        // `create`, not `create_all`: an existing path (a planted one) fails the call.
        std::fs::DirBuilder::new().mode(0o700).create(&dir).map_err(|_| not_run("no private git dir", false))?;
    }
    // From here the guard removes the dir on every return.
    let mut iso = Isolated { dir: dir.clone(), env: Vec::new() };
    let fail = |_| not_run("the private git dir could not be written", false);
    std::fs::write(dir.join("config"), format_config(&fmt)).map_err(fail)?;
    for name in LINKED {
        let real = common.join(name);
        if std::fs::symlink_metadata(&real).is_ok() {
            std::os::unix::fs::symlink(&real, dir.join(name)).map_err(fail)?;
        }
    }
    let mut env: Vec<(String, OsString)> = vec![
        ("GIT_DIR".into(), gitdir.into()),
        ("GIT_COMMON_DIR".into(), dir.into()),
        ("GIT_WORK_TREE".into(), top.into()),
        ("GIT_OBJECT_DIRECTORY".into(), objects.into()),
        ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
        ("GIT_CONFIG_COUNT".into(), pairs.len().to_string().into()),
    ];
    env.extend(ENV.iter().map(|(k, v)| (k.to_string(), OsString::from(v))));
    for (i, (k, v)) in pairs.into_iter().enumerate() {
        env.push((format!("GIT_CONFIG_KEY_{i}"), k.into()));
        env.push((format!("GIT_CONFIG_VALUE_{i}"), v.into()));
    }
    iso.env = env;
    Ok(iso)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_ref_readers_skip_isolation() {
        for s in ["--version", "rev-parse", "merge-base"] {
            assert!(!needs_isolation(s), "{s}");
        }
        for s in ["status", "diff", "log", "ls-files", "show", "", "anything-new"] {
            assert!(needs_isolation(s), "{s}");
        }
    }

    #[test]
    fn only_data_keys_pass() {
        for k in ["core.ignorecase", "core.precomposeUnicode", "core.excludesFile", "branch.topic.remote", "branch.a.b.merge", "remote.origin.fetch"] {
            assert!(passed(k), "{k}");
        }
        for k in [
            "core.fsmonitor", "core.hooksPath", "core.sshCommand", "core.pager", "core.editor", "core.attributesFile", "core.worktree",
            "filter.x.clean", "filter.x.process", "diff.y.textconv", "diff.y.command", "diff.external", "gpg.program", "log.showSignature",
            "include.path", "includeIf.gitdir:/x.path", "remote.origin.uploadpack", "remote.origin.url", "branch.remote", "core.x.ignorecase",
            "alias.st", "credential.helper", "extensions.worktreeConfig", "uploadpack.packObjectsHook",
        ] {
            assert!(!passed(k), "{k}");
        }
    }

    #[test]
    fn parse_keeps_passed_pairs_and_the_format() {
        let out = b"core.ignorecase\ntrue\0filter.x.clean\ntouch /tmp/p\0core.bare\nfalse\0extensions.objectformat\nSHA256\0core.symlinks\0branch.t.merge\nrefs/heads/main\0";
        let (pairs, fmt) = parse(out).unwrap();
        assert_eq!(pairs, [("core.ignorecase".into(), "true".into()), ("core.symlinks".into(), "true".into()), ("branch.t.merge".into(), "refs/heads/main".into())]);
        assert_eq!(fmt.extensions, [("objectformat", "sha256".to_string())]);
        assert_eq!(format_config(&fmt), "[core]\n\trepositoryformatversion = 1\n[extensions]\n\tobjectformat = sha256\n");
        assert_eq!(format_config(&Format::default()), "[core]\n\trepositoryformatversion = 0\n");
    }

    #[test]
    fn parse_refuses_bad_output_and_unknown_formats() {
        assert!(parse(b"core.ignorecase\ntrue").is_none(), "no final NUL");
        assert!(parse(b"extensions.objectformat\nmd5\0").is_none());
        assert!(parse(b"extensions.refstorage\nweird\0").is_none());
        assert!(parse(b"\0").is_none());
        assert_eq!(parse(b"").map(|(p, f)| (p.len(), f)), Some((0, Format::default())));
    }
}
