//! A second app on the same account never kills the first app's live worker (6d2d): the 6c-3 start-up sweep kills
//! processes that started before the app, so the second app here is a fresh process (this test binary run again)
//! that opens after the worker started. Without the worktree folder's instance lock it would sweep that worker.
//! Temp HOME and data folders only; the worker is killed and reaped at the end whatever happens.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const CHILD: &str = "VAJRA_TEST_SECOND_INSTANCE";
const SLEEP: &str = "VAJRA_TEST_SECOND_INSTANCE_SLEEP";

/// The fake worker's body: this test binary itself, since macOS hides a platform binary's environment (and so its
/// TMPDIR, the sweep's mark). Ignored unless asked for.
#[test]
#[ignore]
fn worker_body() {
    if std::env::var_os(SLEEP).is_some() {
        std::thread::sleep(Duration::from_secs(60));
    }
}

/// Run as the second app: open on `data` with `home`, report what the start-up did, then hold until killed.
#[test]
fn second_instance_child() {
    let Some(args) = std::env::var_os(CHILD) else { return };
    let args = args.into_string().unwrap();
    let (data, home) = args.split_once('\n').unwrap();
    let app = vajra_desktop::desktop::App::open(Path::new(data), Path::new(home)).unwrap();
    println!("REPORT {}", app.swept_report());
}

fn tmp() -> PathBuf {
    let d = PathBuf::from(format!("/tmp/vsi-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.canonicalize().unwrap()
}

#[test]
fn a_second_app_with_its_own_data_folder_does_not_sweep_the_first_apps_live_worker() {
    if std::env::var_os(CHILD).is_some() {
        return;
    }
    let d = tmp();
    let home = d.join("h");
    std::fs::create_dir_all(&home).unwrap();
    // The first app, holding the account's worktree folder.
    let first = vajra_desktop::desktop::App::open(&d.join("data-1"), &home).unwrap();
    assert!(!first.other_instance());
    // Its live worker: the ticket TMPDIR under the shared worktree folder.
    let work = vajra_desktop::vajra::gitw::work_root(&home);
    let tmp = work.join("p").join("20261112-0930").join("tmp").join("t1");
    std::fs::create_dir_all(&tmp).unwrap();
    let me = std::env::current_exe().unwrap();
    let mut worker = Command::new(&me)
        .args(["worker_body", "--exact", "--ignored", "--nocapture"])
        .env("TMPDIR", &tmp)
        .env(SLEEP, "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    // Until its TMPDIR is in place (its exec is done), so the second app starts after it.
    std::thread::sleep(Duration::from_millis(500));
    // The second app, a fresh process started after the worker, with its own data folder.
    let out = Command::new(&me)
        .args(["second_instance_child", "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD, format!("{}\n{}", d.join("data-2").display(), home.display()))
        .output()
        .unwrap();
    let alive = worker.try_wait().unwrap().is_none();
    let _ = worker.kill();
    let _ = worker.wait();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let _ = std::fs::remove_dir_all(&d);
    assert!(out.status.success(), "{text}{}", String::from_utf8_lossy(&out.stderr));
    let line = text.lines().find_map(|l| l.split_once("REPORT ").map(|(_, r)| r)).unwrap_or_else(|| panic!("no report from the second app: {text}{}", String::from_utf8_lossy(&out.stderr)));
    let report: serde_json::Value = serde_json::from_str(line).unwrap();
    assert_eq!(report["crash_reconcile"]["other_instance"], true, "{report}");
    assert!(alive, "a running app's worker is not an earlier life's: {report}");
    drop(first);
}
