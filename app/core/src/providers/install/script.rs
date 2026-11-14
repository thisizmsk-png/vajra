//! Vendor install scripts (ADR-17 rule 2): "Install it yourself", because hashing a script doesn't pin what it
//! downloads, unless the vendor documents that the script verifies its own download. Even then it is fetched over https
//! only, its sha256 is checked, and it runs as a file (`/bin/sh <file>`), never piped into a shell or passed to `-c`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::plan::{hex, Pin, Plan, ToolRow};
use crate::providers::catalog;

/// Stands in for the fetched script's path in the shown command.
pub const SCRIPT: &str = "<verified script>";
pub const CURL: &str = "/usr/bin/curl";
const SH: &str = "/bin/sh";

fn https(url: &str) -> bool {
    url.starts_with("https://") && !url.contains(char::is_whitespace)
}

pub fn plan(id: &str, m: &catalog::VendorScript) -> Result<Plan, &'static str> {
    if !https(&m.url) {
        return Err("vendor_script_url");
    }
    if !m.self_verifying {
        return Err("vendor_script_unverified");
    }
    let tools = vec![
        ToolRow { role: "sh", path: SH.into(), label: None, owned_by_you: false },
        ToolRow { role: "curl", path: CURL.into(), label: None, owned_by_you: false },
    ];
    Ok(Plan::new(id, "vendor_script", vec![SH.into(), SCRIPT.into()], tools, m.url.clone(), String::new(), String::new(), Pin::Script { sha256: m.sha256.clone() }))
}

/// curl's arguments: https only, TLS 1.2 or later, fail on an HTTP error, into `dest`.
pub fn curl_argv(url: &str, dest: &Path) -> Vec<OsString> {
    ["--proto", "=https", "--tlsv1.2", "-fsSL", "-o"].iter().map(OsString::from).chain([dest.as_os_str().to_owned(), url.into()]).collect()
}

/// Fetch the script into a fresh private folder with `get(url, dest)` and keep it only when its sha256 is the catalog's.
pub fn fetch(m: &catalog::VendorScript, get: &dyn Fn(&str, &Path) -> Result<(), &'static str>) -> Result<PathBuf, &'static str> {
    if !https(&m.url) {
        return Err("vendor_script_url");
    }
    let dir = crate::providers::setup::private_dir("vajra-script").map_err(|_| "io")?;
    let file = dir.join("install.sh");
    let checked = get(&m.url, &file).and_then(|_| std::fs::read(&file).map_err(|_| "fetch")).and_then(|b| if hex(&Sha256::digest(&b)) == m.sha256 { Ok(()) } else { Err("script_sha256") });
    match checked {
        Ok(()) => Ok(file),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            Err(e)
        }
    }
}

/// The run's argv: the shell and the checked file, nothing else.
pub fn argv(file: &Path) -> Vec<OsString> {
    vec![SH.into(), file.as_os_str().to_owned()]
}

#[cfg(test)]
#[path = "script_tests.rs"]
mod tests;
