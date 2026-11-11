use super::*;

fn method(self_verifying: bool, body: &[u8]) -> catalog::VendorScript {
    catalog::VendorScript { url: "https://vendor.example/install.sh".into(), sha256: super::super::plan::hex(&sha2::Sha256::digest(body)), self_verifying, command: "curl -fsSL https://vendor.example/install.sh | sh".into() }
}

use sha2::Digest;

#[test]
fn a_script_not_documented_to_verify_its_download_is_install_it_yourself() {
    assert_eq!(plan("x", &method(false, b"echo hi")).unwrap_err(), "vendor_script_unverified");
}

#[test]
fn a_self_verifying_script_runs_as_a_file_never_through_a_shell_string() {
    let p = plan("x", &method(true, b"echo hi")).unwrap();
    assert_eq!(p.command, ["/bin/sh", SCRIPT]);
    assert!(!p.command.iter().any(|a| a == "-c" || a.contains('|')));
    assert_eq!(p.method, "vendor_script");
    assert!(matches!(p.pin, Pin::Script { .. }));
}

#[test]
fn the_script_is_hashed_before_it_is_kept_and_a_mismatch_leaves_nothing() {
    let m = method(true, b"echo hi");
    let ok = fetch(&m, &|_, dest: &Path| std::fs::write(dest, b"echo hi").map_err(|_| "fetch")).unwrap();
    assert_eq!(std::fs::read(&ok).unwrap(), b"echo hi");
    assert_eq!(argv(&ok), [std::ffi::OsString::from("/bin/sh"), ok.clone().into_os_string()]);
    let _ = std::fs::remove_dir_all(ok.parent().unwrap());
    let kept = std::cell::RefCell::new(None);
    let bad = fetch(&m, &|_, dest: &Path| {
        *kept.borrow_mut() = Some(dest.to_path_buf());
        std::fs::write(dest, b"rm -rf ~").map_err(|_| "fetch")
    });
    assert_eq!(bad.unwrap_err(), "script_sha256");
    assert!(!kept.into_inner().unwrap().parent().unwrap().exists(), "the folder and the bad script are removed");
}

#[test]
fn only_https_is_ever_fetched() {
    let mut m = method(true, b"echo hi");
    m.url = "http://vendor.example/install.sh".into();
    assert_eq!(plan("x", &m).unwrap_err(), "vendor_script_url");
    assert_eq!(fetch(&m, &|_, _: &Path| Ok(())).unwrap_err(), "vendor_script_url");
}

#[test]
fn curl_is_absolute_and_https_only() {
    let a = curl_argv("https://vendor.example/install.sh", Path::new("/tmp/x/install.sh"));
    assert_eq!(a[0], "--proto");
    assert_eq!(a[1], "=https");
    assert!(a.contains(&"--tlsv1.2".into()));
    assert_eq!(CURL, "/usr/bin/curl");
}
