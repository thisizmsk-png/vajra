//! Red team G2 M1: the Terminal line works on a clean Mac. It installs from the engine bundled in the app, checked
//! against this build's pin, and never names a skills-folder copy that isn't there. The run tests paste the line into
//! a real shell with a temp HOME and a FAKE installer (no real engine installer ever runs here). bhishma on #283: the
//! line itself refuses anything already at the skills folder when it is pasted, not only when it is built.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::super::pack::{raw_tgz, sha256_hex};
use super::*;

const FALLBACK: &str = "bash ~/.claude/skills/vajra/scripts/run-me";

/// A fake installer: records that it ran, from where, and that the files it ships came along. Bash builtins only, as
/// the paste tests give the shell no PATH.
const FAKE: &str = "#!/bin/bash\nset -e\nd=\"$(cd \"${0%/*}/..\" && pwd)\"\n[ -f \"$d/workflows/vajra-deliver.js\" ]\necho \"ran $d\" >> \"$HOME/installer-ran\"\n";

struct World {
    root: PathBuf,
    home: PathBuf,
    res: PathBuf,
    pin: Pin,
}

impl World {
    /// A temp home and an app bundle whose path has spaces, as `/Applications/Vajra Desktop.app` does.
    fn new(tag: &str) -> World {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!("vajra-manual-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (home, res) = (root.join("home"), root.join("Vajra Desktop.app/Contents/Resources"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(res.join("engine")).unwrap();
        let files: Vec<(String, Vec<u8>, u32)> = vec![
            ("package.json".into(), br#"{"version":"0.6.2"}"#.to_vec(), 0o644),
            (INSTALLER.into(), FAKE.as_bytes().to_vec(), 0o755),
            ("workflows/vajra-deliver.js".into(), b"// deliver".to_vec(), 0o644),
        ];
        let m: serde_json::Map<String, serde_json::Value> = files.iter().map(|(n, b, _)| (n.clone(), sha256_hex(b).into())).collect();
        let mut all = vec![("manifest.json".to_string(), serde_json::to_vec(&serde_json::json!({ "files": m })).unwrap(), 0o644)];
        all.extend(files);
        let tgz = raw_tgz(&all, b'0');
        std::fs::write(res.join(RESOURCE), &tgz).unwrap();
        let pin = Pin { sha256: sha256_hex(&tgz), version: "0.6.2".into(), tree_ok: true, unpacked: 0, files: all.len(), release: true };
        World { root, home, res, pin }
    }

    fn line(&self) -> Line {
        line(&self.home, Some(&self.res), &self.pin, FALLBACK)
    }

    fn command(&self) -> String {
        self.line().command
    }

    fn skill(&self) -> PathBuf {
        self.home.join(".claude/skills/vajra")
    }

    /// Paste `line` into `shell` with this world's HOME and an empty PATH (the line names every tool by its absolute
    /// path). None when the shell isn't on this machine.
    fn paste(&self, shell: &str, line: &str) -> Option<std::process::Output> {
        Path::new(shell).is_file().then(|| {
            std::process::Command::new(shell)
                .args(["-f", "-c", line])
                .env_clear()
                .env("HOME", &self.home)
                .env("PATH", self.root.join("no-tools"))
                .current_dir(&self.root)
                .output()
                .unwrap()
        })
    }

    /// Every file under `dir` (not following links) with its bytes, and every link with its target.
    fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut out = BTreeMap::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                let t = std::fs::symlink_metadata(&p).unwrap().file_type();
                if t.is_symlink() {
                    out.insert(p.clone(), std::fs::read_link(&p).unwrap().into_os_string().into_encoded_bytes());
                } else if t.is_dir() {
                    stack.push(p);
                } else {
                    out.insert(p.clone(), std::fs::read(&p).unwrap());
                }
            }
        }
        out
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a_clean_mac_gets_a_line_that_installs_from_the_bundled_engine() {
    let w = World::new("shape");
    let l = w.line();
    assert!(!l.leftover);
    let c = l.command;
    let tgz = w.res.join(RESOURCE).display().to_string();
    assert!(c.starts_with(&format!("echo '{}  {tgz}' | /usr/bin/shasum -a 256 -c && ", w.pin.sha256)), "{c}");
    assert!(c.contains(" && /bin/mkdir -p ~/.claude/skills && /bin/mkdir ~/.claude/skills/vajra && "), "the target is made with a plain mkdir: {c}");
    assert!(c.contains(&format!("/usr/bin/tar -xzf '{tgz}' -C ~/.claude/skills/vajra && ")), "{c}");
    assert!(c.ends_with(&format!(" && /bin/bash ~/.claude/skills/vajra/{INSTALLER}")), "{c}");
    assert!(!c.contains('\n') && !c.contains("git clone") && !c.contains("/vajra doctor"), "{c}");
    // Every tool by its absolute system path: nothing is found through PATH (echo is the shell's own).
    for tool in ["shasum", "mkdir", "tar", "bash"] {
        assert_eq!(c.matches(tool).count(), c.matches(&format!("/{tool}")).count(), "{tool} by path only: {c}");
    }
    assert!(c.len() <= MAX_COMMAND);
}

/// The longest bundle path whose line fits the setup list, under a real app location. Shared with the UI test.
fn longest_fitting_path() -> String {
    let at = |k: usize| format!("/Users/someone/Applications/{}/Vajra Desktop.app/Contents/Resources/{RESOURCE}", "d".repeat(k));
    let sha = "a".repeat(64);
    let k = (0..2048).take_while(|&k| command(&at(k), &sha).is_some()).last().unwrap();
    at(k)
}

/// Codex P2 on #283: the core never sends an engine command the UI's setup-list parser refuses. A path one byte
/// longer than the longest that fits gets the catalog line; the longest that fits is byte for byte the golden line
/// `app/ui/tests/setup.test.tsx` passes through `parseSetupList`.
#[test]
fn the_longest_bundled_line_fits_the_setup_list_and_one_byte_more_falls_back() {
    let sha = "a".repeat(64);
    let path = longest_fitting_path();
    let longest = command(&path, &sha).unwrap();
    assert!(longest.len() <= MAX_COMMAND && longest.len() > MAX_COMMAND - 2, "{}", longest.len());
    let longer = path.replacen("/Applications/", "/Applications/d", 1);
    assert_eq!(command(&longer, &sha), None);
    // A pinned bundle that really sits at a path too deep for the line: the catalog line, not a list the UI drops.
    let w = World::new("deep");
    let deep = w.root.join("d".repeat(250)).join("e".repeat(250));
    std::fs::create_dir_all(deep.join("engine")).unwrap();
    std::fs::copy(w.res.join(RESOURCE), deep.join(RESOURCE)).unwrap();
    assert_eq!(line(&w.home, Some(&deep), &w.pin, FALLBACK).command, FALLBACK);
    let golden: serde_json::Value = serde_json::from_str(include_str!("../../../../tests/fixtures/setup/engine-longest-line.json")).unwrap();
    assert_eq!(golden["command"], longest.as_str(), "regenerate the fixture from this test if the line's shape changes");
}

#[test]
fn pasting_the_line_unpacks_the_checked_engine_and_runs_its_installer() {
    let w = World::new("run");
    let mut ran = 0;
    for shell in ["/bin/zsh", "/bin/bash"] {
        let _ = std::fs::remove_dir_all(w.home.join(".claude"));
        let _ = std::fs::remove_file(w.home.join("installer-ran"));
        let Some(out) = w.paste(shell, &w.command()) else { continue };
        assert!(out.status.success(), "{shell}: {}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        let log = std::fs::read_to_string(w.home.join("installer-ran")).unwrap();
        assert_eq!(log.trim(), format!("ran {}", w.skill().display()), "{shell}");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(w.skill().join(INSTALLER)).unwrap().permissions().mode() & 0o777, 0o755);
        assert!(w.skill().join("manifest.json").is_file());
        ran += 1;
    }
    assert!(ran > 0, "no shell to paste into");
}

#[test]
fn a_bundle_that_is_not_the_pinned_one_installs_nothing() {
    let mut w = World::new("tampered");
    w.pin.sha256 = "0".repeat(64);
    let Some(out) = w.paste("/bin/bash", &w.command()) else { return };
    assert!(!out.status.success());
    assert!(!w.home.join(".claude").exists(), "nothing is unpacked from a bundle that fails the check");
    assert!(!w.home.join("installer-ran").exists());
}

/// bhishma MUST-1 on #283: the line is built on a clean Mac, then pasted after something appeared at the skills folder
/// (a later paste from history, or the same line on another Mac). The line stops at its plain `mkdir`: nothing is
/// unpacked, the installer never runs, and every file there (or behind a symlink) is byte for byte as it was.
#[test]
fn a_line_pasted_after_something_appeared_at_the_skills_folder_stops_and_changes_nothing() {
    type Plant = fn(&World);
    let cases: [(&str, Plant); 5] = [
        ("a git checkout", |w| {
            std::fs::create_dir_all(w.skill().join(".git")).unwrap();
            std::fs::create_dir_all(w.skill().join("workflows")).unwrap();
            std::fs::write(w.skill().join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
            std::fs::write(w.skill().join("workflows/vajra-deliver.js"), "// my local edits").unwrap();
        }),
        ("a copy with its own installer", |w| {
            std::fs::create_dir_all(w.skill().join("scripts")).unwrap();
            std::fs::write(w.skill().join(INSTALLER), "#!/bin/bash\necho mine\n").unwrap();
        }),
        ("a symlink to a checkout", |w| {
            let real = w.root.join("my-checkout");
            std::fs::create_dir_all(real.join("workflows")).unwrap();
            std::fs::write(real.join("workflows/vajra-deliver.js"), "// mine").unwrap();
            std::fs::create_dir_all(w.skill().parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(&real, w.skill()).unwrap();
        }),
        ("a dangling symlink", |w| {
            std::fs::create_dir_all(w.skill().parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(w.root.join("gone"), w.skill()).unwrap();
        }),
        ("a plain file", |w| {
            std::fs::create_dir_all(w.skill().parent().unwrap()).unwrap();
            std::fs::write(w.skill(), "not a folder").unwrap();
        }),
    ];
    for shell in ["/bin/bash", "/bin/zsh"] {
        for (what, plant) in cases {
            let w = World::new(&format!("late-{}-{}", shell.rsplit('/').next().unwrap(), what.replace(' ', "-")));
            let line = w.command();
            assert!(line.starts_with("echo '"), "built on a clean Mac: {line}");
            plant(&w);
            let before = World::snapshot(&w.root);
            let Some(out) = w.paste(shell, &line) else { continue };
            assert!(!out.status.success(), "{shell}, {what}: the line must stop");
            assert!(!w.home.join("installer-ran").exists(), "{shell}, {what}: no installer ran");
            assert_eq!(World::snapshot(&w.root), before, "{shell}, {what}: nothing changed");
            assert_eq!(w.command(), FALLBACK, "{what}: built now, the line is the catalog's");
        }
    }
}

#[test]
fn anything_at_the_skills_folder_keeps_the_catalog_line_and_a_folder_with_no_installer_is_a_leftover() {
    let w = World::new("copy");
    std::fs::create_dir_all(w.skill().join("scripts")).unwrap();
    std::fs::write(w.skill().join(INSTALLER), "#!/bin/bash\n").unwrap();
    assert_eq!(w.line(), Line { command: FALLBACK.into(), leftover: false }, "a copy runs its own installer");
    std::fs::remove_file(w.skill().join(INSTALLER)).unwrap();
    assert_eq!(w.line(), Line { command: FALLBACK.into(), leftover: true }, "a half-made folder is never unpacked over");
    std::fs::remove_dir_all(w.skill()).unwrap();
    let real = w.root.join("checkout");
    std::fs::create_dir_all(real.join("scripts")).unwrap();
    std::fs::write(real.join(INSTALLER), "#!/bin/bash\n").unwrap();
    std::os::unix::fs::symlink(&real, w.skill()).unwrap();
    assert_eq!(w.line(), Line { command: FALLBACK.into(), leftover: false }, "a symlinked checkout runs its own installer");
    std::fs::remove_file(w.skill()).unwrap();
    std::os::unix::fs::symlink(w.root.join("gone"), w.skill()).unwrap();
    assert_eq!(w.line(), Line { command: FALLBACK.into(), leftover: true }, "a dangling symlink is a leftover");
    std::fs::remove_file(w.skill()).unwrap();
    assert!(w.command().starts_with("echo '"), "gone again: the bundled line");
}

#[test]
fn no_pinned_bundle_falls_back_to_the_catalog_line() {
    let mut w = World::new("none");
    assert_eq!(line(&w.home, None, &w.pin, FALLBACK).command, FALLBACK, "no resources folder yet");
    assert_eq!(line(&w.home, Some(&w.root.join("nowhere")), &w.pin, FALLBACK).command, FALLBACK, "no bundle");
    w.pin.tree_ok = false;
    assert_eq!(w.command(), FALLBACK, "an unpinned build");
    w.pin.tree_ok = true;
    w.pin.sha256 = "not-a-hash".into();
    assert_eq!(w.command(), FALLBACK);
    assert!(!w.line().leftover);
}

#[test]
fn a_symlinked_bundle_or_an_unquotable_path_falls_back() {
    let w = World::new("odd");
    let real = w.res.join(RESOURCE);
    let moved = w.root.join("elsewhere.tgz");
    std::fs::rename(&real, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &real).unwrap();
    assert_eq!(w.command(), FALLBACK, "a symlinked bundle");
    // bhishma LATER-5 on #283: Unicode line and paragraph separators, bidi controls and zero-width marks too.
    for bad in ["it's", "new\nline", "back\\slash", "nel\u{85}x", "ls\u{2028}x", "ps\u{2029}x", "rlo\u{202e}x", "lre\u{202a}x", "rli\u{2067}x", "pdi\u{2069}x", "zw\u{200b}x", "bom\u{feff}x"] {
        let res = w.root.join(bad);
        std::fs::create_dir_all(res.join("engine")).unwrap();
        std::fs::copy(&moved, res.join(RESOURCE)).unwrap();
        assert_eq!(line(&w.home, Some(&res), &w.pin, FALLBACK).command, FALLBACK, "{bad:?}");
    }
    // A plain non-ASCII folder name is fine.
    let res = w.root.join("Programme für José");
    std::fs::create_dir_all(res.join("engine")).unwrap();
    std::fs::copy(&moved, res.join(RESOURCE)).unwrap();
    assert!(line(&w.home, Some(&res), &w.pin, FALLBACK).command.starts_with("echo '"));
}
