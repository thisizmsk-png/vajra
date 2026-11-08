//! #70 review (NFR-SEC-2): the bridge runs `vajra-campaign.sh`, the only holder of the HMAC key, which calls `openssl`,
//! `jq`, `awk`, `tr` and `mktemp` by bare name. A same-uid worker can write `~/.local/bin` (or any user dir a login
//! shell puts first), so no bridged program may ever resolve a tool from a user dir ahead of the system dirs.
//!
//! Its own test binary: `userpath::init` is once per process, and this test resolves the user PATH from a temp HOME
//! whose login profile puts the planted dir first, exactly as a real `~/.zprofile` can.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use vajra_desktop::vajra::bridge::{Bridge, Request, run_process_on_path};

fn exe(p: &Path, body: &str) {
    fs::write(p, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(p, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn a_planted_openssl_or_jq_in_a_user_dir_is_never_run_by_the_bridge() {
    let root = std::env::temp_dir().join(format!("vajra-bridge-path-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let home = root.join("home");
    let planted = home.join(".local/bin");
    fs::create_dir_all(&planted).unwrap();
    let home = home.canonicalize().unwrap();
    let planted = planted.canonicalize().unwrap();
    // The worker's trap: a fake openssl and jq that record their argv (where the key would be).
    let caught = root.join("caught");
    for tool in ["openssl", "jq", "awk", "tr", "mktemp"] {
        exe(&planted.join(tool), &format!("echo \"{tool} $*\" >> '{}'", caught.display()));
    }
    // A login profile that puts the planted dir first, for whichever shell the user has.
    for rc in [".zprofile", ".zshrc", ".bash_profile", ".bashrc", ".profile"] {
        fs::write(home.join(rc), "export PATH=\"$HOME/.local/bin:$PATH\"\n").unwrap();
    }
    vajra_desktop::userpath::init(&home);
    assert!(vajra_desktop::userpath::current().contains(&planted), "the test needs the planted dir in the user PATH");

    // A key-holding script stand-in: it calls the tools by bare name, as vajra-campaign.sh does.
    let scripts = root.join("scripts");
    fs::create_dir_all(&scripts).unwrap();
    exe(
        &scripts.join("vajra-campaign.sh"),
        "echo \"PATH=$PATH\"; for t in openssl jq awk tr mktemp; do echo \"$t=$(command -v $t)\"; done; \
         printf x | openssl dgst -sha256 -hmac SECRETKEY >/dev/null 2>&1; echo '{}' | jq . >/dev/null 2>&1; exit 0",
    );
    exe(&scripts.join("vajra-doctor.sh"), "echo \"PATH=$PATH\"");
    let b = Bridge::new(scripts.clone()).with_home(home.clone());
    for req in [Request::CampaignVerify { slug: "deliver-x".into() }, Request::VerifyDigest { input: b"a\nb\nc\nd\n".to_vec() }, Request::Doctor] {
        let out = b.run(&req).unwrap();
        let path = out.stdout.lines().find_map(|l| l.strip_prefix("PATH=")).unwrap().to_string();
        assert_eq!(path, "/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/usr/local/bin", "{req:?}: the bridge PATH is the fixed system one");
        for l in out.stdout.lines().filter(|l| l.contains('=') && !l.starts_with("PATH=")) {
            let (_, at) = l.split_once('=').unwrap();
            assert!(!at.starts_with(&*planted.to_string_lossy()), "{req:?}: {l}");
        }
    }
    assert!(!caught.exists(), "a planted tool ran: {}", fs::read_to_string(&caught).unwrap_or_default());

    // The claude version check may see user dirs (an npm claude needs its node), but only after the system dirs, and it
    // is never given the key.
    let probe = scripts.join("probe.sh");
    exe(&probe, "echo \"PATH=$PATH\"; echo \"openssl=$(command -v openssl)\"; openssl version >/dev/null 2>&1; exit 0");
    let out = run_process_on_path(&probe, &[], None, Duration::from_secs(10), std::slice::from_ref(&planted)).unwrap();
    let path = out.stdout.lines().find_map(|l| l.strip_prefix("PATH=")).unwrap().to_string();
    let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    assert_eq!(dirs[0], PathBuf::from("/usr/bin"), "system dirs first: {dirs:?}");
    assert_eq!(dirs.last(), Some(&planted), "user dirs only after them: {dirs:?}");
    assert!(!caught.exists(), "a planted tool ran from the claude check: {}", fs::read_to_string(&caught).unwrap_or_default());
    let _ = fs::remove_dir_all(&root);
}
