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
//! So every bridged git call but `--version` (`needs_isolation`) runs `isolate`d: the repository's own config files
//! are never read by it.
//! - One `rev-parse` finds the git dir, common dir and top level, and one `config --list` reads the effective config
//!   (system file off, as on every bridged call). Neither reads a working file, an object or an index, so neither runs
//!   a filter, driver, hook or fetch.
//! - From that one read, only `PASSED` keys go on, as command-scope `GIT_CONFIG_KEY_<n>` pairs: data the output
//!   depends on (case, unicode and mode handling, line endings, excludes, a branch's upstream). Never a key naming a
//!   program, so no filter, driver, hook, signer or transport is defined at all; an attribute naming one does nothing.
//! - The command then runs with `GIT_COMMON_DIR` set to a private dir (mode 0700, removed afterwards) whose `config`
//!   holds only the repository format, written create-new and no-follow. The dir is made in a folder under the app's
//!   data dir, which no agent sandbox may write; never in the temp folder, which a sandbox may (SECURITY.md). Its
//!   `refs`, `packed-refs`, `objects`, `info`, `logs`, `shallow` and
//!   `reftable` are links to the real ones. `GIT_DIR` is the real git dir (`HEAD`, the index) and `GIT_WORK_TREE` the
//!   real top level. Git reads the local config from the common dir, and `config.worktree` only when that config
//!   sets `extensions.worktreeConfig`, which it never does; `GIT_CONFIG_GLOBAL=/dev/null` and `GIT_CONFIG_NOSYSTEM=1`
//!   leave out the other files. A config written after the one read is never seen.
//! - Every call also gets `HARDENING` (`-c`, outranking any file) and `ENV`: no optional index write, no lazy fetch,
//!   no system attributes file, no transport at all.
//! - The folder asked about must be inside the top level git reports, so a moved work tree is refused.
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
/// `GIT_ALLOW_PROTOCOL` names no real protocol: it outranks `protocol.allow` and every per-protocol key in any config,
/// every supported git honours it, and a child git (a lazy fetch) inherits it.
pub const ENV: [(&str, &str); 4] = [("GIT_OPTIONAL_LOCKS", "0"), ("GIT_NO_LAZY_FETCH", "1"), ("GIT_ATTR_NOSYSTEM", "1"), ("GIT_ALLOW_PROTOCOL", "none")];

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

/// Whether a bridged subcommand runs `isolate`d: every one but `--version`, which reads no repository. `rev-parse` and
/// `merge-base` too (bhishma and Claude on #299): a missing commit in a partial clone starts a lazy fetch, which runs the
/// repo's `remote.<name>.uploadpack` over a protocol a per-protocol `allow` key opens, and `GIT_NO_LAZY_FETCH` exists only
/// from git 2.44. The private config names no promisor, so no git version fetches.
pub fn needs_isolation(sub: &str) -> bool {
    sub != "--version"
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

/// Write `bytes` to a new file at `path`: create-new and no-follow, so an existing file or a planted link (even a
/// dangling one) fails the write instead of being written through.
pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(path)?;
    f.write_all(bytes)
}

/// The folder the private common dirs are made in, `scratch`, created at mode 0700 if missing. It must be a real
/// directory (not a link) owned by this user. The caller passes one under the app's data dir, which no agent sandbox
/// may write (bhishma on #299: a sandbox may write the temp folder, and could plant the private `config` there).
fn scratch_root(scratch: &Path) -> Option<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    match std::fs::DirBuilder::new().mode(0o700).create(scratch) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return None,
    }
    let md = std::fs::symlink_metadata(scratch).ok()?;
    // SAFETY: geteuid has no preconditions.
    (md.file_type().is_dir() && md.uid() == unsafe { libc::geteuid() } && md.mode() & 0o022 == 0).then_some(())
}

/// Set up an isolated call for `cwd` (module doc), within `deadline`, with its private dir made inside `scratch`
/// (`None`: no such folder is configured, and nothing runs). `Err` holds what the caller should see instead of the
/// command's output: the failing step's own output (`cwd` is not in a work tree: git's exit 128 and message), or
/// `not_run`.
pub fn isolate(git: &Path, cwd: &Path, home: Option<&Path>, scratch: Option<&Path>, deadline: Instant) -> Result<Isolated, Output> {
    let scratch = scratch.ok_or_else(|| not_run("no private git folder is configured", false))?;
    scratch_root(scratch).ok_or_else(|| not_run("the private git folder is not a private directory", false))?;
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
    // A work tree moved elsewhere (`core.worktree`, a `GIT_WORK_TREE` in config) would point the call at files the
    // caller never named: the folder asked about must be inside the top level git reports.
    if !cwd.canonicalize().is_ok_and(|c| c.starts_with(&top)) {
        return Err(not_run("the repository's work tree is not the folder asked about", false));
    }
    let objects = common.join("objects").canonicalize().ok().filter(|p| p.is_dir()).ok_or_else(|| not_run("the repository has no object store", false))?;
    let listed = step(&["config", "--list", "-z"])?;
    if listed.truncated {
        return Err(not_run("the repository config is too large", false));
    }
    let (pairs, fmt) = parse(&listed.stdout_raw).ok_or_else(|| not_run("the repository config could not be read", false))?;

    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    let dir = scratch.join(format!("vajra-git-{}-{}-{nanos}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
    {
        use std::os::unix::fs::DirBuilderExt;
        // `create`, not `create_all`: an existing path (a planted one) fails the call.
        std::fs::DirBuilder::new().mode(0o700).create(&dir).map_err(|_| not_run("no private git dir", false))?;
    }
    // From here the guard removes the dir on every return.
    let mut iso = Isolated { dir: dir.clone(), env: Vec::new() };
    let fail = |_| not_run("the private git dir could not be written", false);
    write_new(&dir.join("config"), format_config(&fmt).as_bytes()).map_err(fail)?;
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
    use std::os::unix::fs::PermissionsExt;

    fn tmp(name: &str) -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!("vajra-gitsafe-unit-{name}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    #[test]
    fn the_private_config_is_never_written_through_a_planted_link() {
        let d = tmp("link");
        let target = d.join("agent-owned");
        std::fs::write(&target, "[core]\n").unwrap();
        std::os::unix::fs::symlink(&target, d.join("config")).unwrap();
        assert!(write_new(&d.join("config"), b"[filter \"x\"]\n").is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"[core]\n", "written through the link");
        // A dangling link too: create-new does not create its target.
        std::os::unix::fs::symlink(d.join("not-yet"), d.join("config2")).unwrap();
        assert!(write_new(&d.join("config2"), b"x").is_err());
        assert!(!d.join("not-yet").exists());
        // An existing plain file is not replaced either.
        std::fs::write(d.join("config3"), "old").unwrap();
        assert!(write_new(&d.join("config3"), b"new").is_err());
        assert_eq!(std::fs::read(d.join("config3")).unwrap(), b"old");
        write_new(&d.join("fresh"), b"ok").unwrap();
        assert_eq!(std::fs::metadata(d.join("fresh")).unwrap().permissions().mode() & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_scratch_folder_must_be_a_private_directory_of_this_user() {
        let d = tmp("root");
        assert!(scratch_root(&d.join("made")).is_some(), "made when missing");
        assert_eq!(std::fs::metadata(d.join("made")).unwrap().permissions().mode() & 0o777, 0o700);
        std::os::unix::fs::symlink(d.join("made"), d.join("linked")).unwrap();
        assert!(scratch_root(&d.join("linked")).is_none(), "a link is refused");
        std::fs::create_dir(d.join("open")).unwrap();
        std::fs::set_permissions(d.join("open"), std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(scratch_root(&d.join("open")).is_none(), "a group- or world-writable folder is refused");
        std::fs::write(d.join("file"), "").unwrap();
        assert!(scratch_root(&d.join("file")).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn with_no_scratch_folder_nothing_runs() {
        let d = tmp("none");
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        let e = isolate(Path::new("/usr/bin/git"), &d, None, None, deadline).unwrap_err();
        assert_eq!(e.code, Some(128));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// bhishma MUST 2 and Claude on #299: on git before 2.44 (no `GIT_NO_LAZY_FETCH`), a partial clone with a planted
    /// `uploadpack`, a per-protocol allow key and a ref to a missing commit made `rev-parse` and `merge-base` run the
    /// program. Simulated here on any git by leaving that variable out.
    #[test]
    fn a_lazy_fetch_never_runs_a_planted_upload_pack_on_older_git() {
        let d = tmp("lazy");
        let (repo, remote, marks, home) = (d.join("repo"), d.join("remote"), d.join("marks"), d.join("home"));
        for p in [&repo, &remote, &marks, &home] {
            std::fs::create_dir_all(p).unwrap();
        }
        let fixture = |dir: &Path, args: &[&str]| {
            let o = std::process::Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=main"]).args(args)
                .current_dir(dir).env("HOME", &home).env("GIT_CONFIG_NOSYSTEM", "1").output().unwrap();
            assert!(o.status.success(), "{args:?}: {}", String::from_utf8_lossy(&o.stderr));
        };
        fixture(&remote, &["init", "-q"]);
        fixture(&repo, &["init", "-q"]);
        fixture(&repo, &["commit", "-q", "--allow-empty", "-m", "base"]);
        let up = d.join("up.sh");
        std::fs::write(&up, format!("#!/bin/sh\ntouch '{}/uploadpack'\nexit 1\n", marks.display())).unwrap();
        std::fs::set_permissions(&up, std::fs::Permissions::from_mode(0o755)).unwrap();
        for (k, v) in [
            ("core.repositoryformatversion", "1"),
            ("extensions.partialclone", "origin"),
            ("remote.origin.url", remote.to_str().unwrap()),
            ("remote.origin.promisor", "true"),
            ("remote.origin.uploadpack", up.to_str().unwrap()),
            ("protocol.file.allow", "always"),
        ] {
            fixture(&repo, &["config", k, v]);
        }
        std::fs::write(repo.join(".git/refs/heads/ghost"), "1234567890123456789012345678901234567890\n").unwrap();
        let marked = || std::fs::read_dir(&marks).unwrap().count() > 0;
        fn older_git<'a>(env: &[(&'a str, &'a Path)]) -> Vec<(&'a str, &'a Path)> {
            env.iter().filter(|(k, _)| !matches!(*k, "GIT_NO_LAZY_FETCH" | "GIT_ALLOW_PROTOCOL")).copied().collect()
        }
        let git = Path::new("/usr/bin/git");
        let calls: [&[&str]; 2] = [&["rev-parse", "--verify", "--quiet", "ghost^{commit}"], &["merge-base", "HEAD", "ghost"]];
        let run = |args: &[&str], env: &[(&str, &Path)]| {
            let argv: Vec<String> = prefix().into_iter().chain(args.iter().map(|a| a.to_string())).collect();
            run_process_env(git, &argv, Some(&repo), std::time::Duration::from_secs(10), Some(&home), None, env).unwrap()
        };
        // Control: hardened only, as on older git, the fixture fires.
        run(calls[0], &[]);
        assert!(marked(), "the fixture did not lazy-fetch");
        std::fs::remove_dir_all(&marks).unwrap();
        std::fs::create_dir(&marks).unwrap();
        // Hardened only, but with `GIT_ALLOW_PROTOCOL`: no transport.
        for args in calls {
            run(args, &[("GIT_ALLOW_PROTOCOL", Path::new("none"))]);
        }
        assert!(!marked(), "GIT_ALLOW_PROTOCOL did not stop the fetch");
        // Isolated, without either variable: the private config names no promisor, so nothing is fetched.
        let scratch = d.join("scratch");
        for args in calls {
            let iso = isolate(git, &repo, Some(&home), Some(&scratch), Instant::now() + std::time::Duration::from_secs(10)).unwrap();
            let o = run(args, &older_git(&iso.env()));
            assert_ne!(o.exit(), Some(0), "{args:?} resolved a missing commit");
        }
        assert!(!marked(), "an isolated call lazy-fetched");
        assert_eq!(std::fs::read_dir(&scratch).unwrap().count(), 0, "a private dir was left behind");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn only_version_skips_isolation() {
        assert!(!needs_isolation("--version"));
        for s in ["rev-parse", "merge-base", "status", "diff", "log", "ls-files", "show", "", "anything-new"] {
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
