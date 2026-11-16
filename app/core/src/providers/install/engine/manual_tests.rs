//! Red team G2 M1: the Terminal line works on a clean Mac. It installs from the engine bundled in the app, checked
//! against this build's pin, and never names a skills-folder copy that isn't there. The run tests paste the line into
//! a real shell with a temp HOME and a FAKE installer (no real engine installer ever runs here).

use std::path::{Path, PathBuf};

use super::super::pack::{raw_tgz, sha256_hex};
use super::*;

const FALLBACK: &str = "bash ~/.claude/skills/vajra/scripts/run-me";

/// A fake installer: records that it ran, from where, and that the files it ships came along.
const FAKE: &str = "#!/bin/bash\nset -e\nd=\"$(cd \"$(dirname \"$0\")/..\" && pwd)\"\n[ -f \"$d/workflows/vajra-deliver.js\" ]\necho \"ran $d\" >> \"$HOME/installer-ran\"\n";

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

    fn line(&self) -> String {
        line(&self.home, Some(&self.res), &self.pin, FALLBACK)
    }

    fn skill(&self) -> PathBuf {
        self.home.join(".claude/skills/vajra")
    }

    /// Paste `line` into `shell` with this world's HOME. None when the shell isn't on this machine.
    fn paste(&self, shell: &str, line: &str) -> Option<std::process::Output> {
        Path::new(shell).is_file().then(|| {
            std::process::Command::new(shell)
                .args(["-f", "-c", line])
                .env_clear()
                .env("HOME", &self.home)
                .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
                .current_dir(&self.root)
                .output()
                .unwrap()
        })
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
    let tgz = w.res.join(RESOURCE).display().to_string();
    assert!(l.starts_with(&format!("echo '{}  {tgz}' | shasum -a 256 -c && ", w.pin.sha256)), "{l}");
    assert!(l.contains(&format!("tar -xzf '{tgz}' -C ~/.claude/skills/vajra")), "{l}");
    assert!(l.ends_with(&format!("bash ~/.claude/skills/vajra/{INSTALLER}")), "{l}");
    assert!(!l.contains('\n') && !l.contains("git clone") && !l.contains("/vajra doctor"), "{l}");
}

#[test]
fn pasting_the_line_unpacks_the_checked_engine_and_runs_its_installer() {
    let w = World::new("run");
    let mut ran = 0;
    for shell in ["/bin/zsh", "/bin/bash"] {
        let _ = std::fs::remove_dir_all(w.home.join(".claude"));
        let _ = std::fs::remove_file(w.home.join("installer-ran"));
        let Some(out) = w.paste(shell, &w.line()) else { continue };
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
    let Some(out) = w.paste("/bin/bash", &w.line()) else { return };
    assert!(!out.status.success());
    assert!(!w.home.join(".claude").exists(), "nothing is unpacked from a bundle that fails the check");
    assert!(!w.home.join("installer-ran").exists());
}

#[test]
fn an_existing_copy_or_checkout_keeps_its_own_installer_line() {
    let w = World::new("copy");
    std::fs::create_dir_all(w.skill().join("scripts")).unwrap();
    std::fs::write(w.skill().join(INSTALLER), "#!/bin/bash\n").unwrap();
    assert_eq!(w.line(), FALLBACK, "a copy that is there runs its own installer, and is never overwritten");
    std::fs::remove_file(w.skill().join(INSTALLER)).unwrap();
    std::fs::create_dir(w.skill().join(".git")).unwrap();
    assert_eq!(w.line(), FALLBACK, "a git checkout is never unpacked over");
    std::fs::remove_dir(w.skill().join(".git")).unwrap();
    assert_ne!(w.line(), FALLBACK, "a half-made folder with no installer gets the bundled line");
}

#[test]
fn no_pinned_bundle_falls_back_to_the_catalog_line() {
    let mut w = World::new("none");
    assert_eq!(line(&w.home, None, &w.pin, FALLBACK), FALLBACK, "no resources folder yet");
    assert_eq!(line(&w.home, Some(&w.root.join("nowhere")), &w.pin, FALLBACK), FALLBACK, "no bundle");
    w.pin.tree_ok = false;
    assert_eq!(w.line(), FALLBACK, "an unpinned build");
    w.pin.tree_ok = true;
    w.pin.sha256 = "not-a-hash".into();
    assert_eq!(w.line(), FALLBACK);
}

#[test]
fn a_symlinked_bundle_or_an_unquotable_path_falls_back() {
    let w = World::new("odd");
    let real = w.res.join(RESOURCE);
    let moved = w.root.join("elsewhere.tgz");
    std::fs::rename(&real, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &real).unwrap();
    assert_eq!(w.line(), FALLBACK, "a symlinked bundle");
    for bad in ["it's", "new\nline", "back\\slash"] {
        let res = w.root.join(bad);
        std::fs::create_dir_all(res.join("engine")).unwrap();
        std::fs::copy(&moved, res.join(RESOURCE)).unwrap();
        assert_eq!(line(&w.home, Some(&res), &w.pin, FALLBACK), FALLBACK, "{bad:?}");
    }
}
