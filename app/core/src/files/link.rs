//! Open a web link from a rendered Markdown file in the person's browser, after the UI has asked them and shown the
//! whole address. The core takes only an absolute `http` or `https` address with a host and no credentials, made of
//! printable ASCII (an internationalised address arrives already punycode/percent-encoded from the URL parser). It is
//! handed to `/usr/bin/open` as one argv element after `--`, never through a shell, so it can't name a file, an app or
//! a custom scheme (`file:`, `javascript:`, `vscode:` and the like are refused here, not just in the UI).

/// Longest address accepted.
pub const LINK_MAX: usize = 2048;

/// Why a link is refused, or `Ok` with the address to open.
pub fn check_link(url: &str) -> Result<&str, &'static str> {
    if url.is_empty() || url.len() > LINK_MAX {
        return Err("the address is empty or too long");
    }
    if !url.bytes().all(|b| (0x21..0x7f).contains(&b)) {
        return Err("the address has spaces, control or non-ASCII characters");
    }
    let rest = if let Some(r) = url.strip_prefix("https://") {
        r
    } else if let Some(r) = url.strip_prefix("http://") {
        r
    } else {
        return Err("only http and https addresses are opened");
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return Err("the address carries a user name or password");
    }
    let host = if authority.starts_with('[') {
        authority.split(']').next().map(|h| h.trim_start_matches('[')).unwrap_or("")
    } else {
        authority.rsplit_once(':').map_or(authority, |(h, _)| h)
    };
    if host.is_empty() || host.starts_with('-') || host.starts_with('.') {
        return Err("the address has no host");
    }
    if !host.bytes().all(|b| b.is_ascii_alphanumeric() || b"-.:".contains(&b)) {
        return Err("the address's host is not a plain host name");
    }
    Ok(url)
}

/// Open `url` (already checked) in the default browser.
pub fn open_link(url: &str) -> std::io::Result<()> {
    let url = check_link(url).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let mut cmd = std::process::Command::new("/usr/bin/open");
    cmd.arg("--").arg(url).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    let mut child = crate::spawnlock::spawn(&mut cmd)?;
    // `open` returns at once; reap it off the caller's thread.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_web_addresses_pass() {
        for u in ["https://example.com", "https://example.com/a/b?c=d#e", "http://localhost:8080/x", "https://[::1]:3000/", "https://xn--bcher-kva.example/"] {
            assert_eq!(check_link(u), Ok(u), "{u}");
        }
    }

    #[test]
    fn other_schemes_files_and_apps_are_refused() {
        for u in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,x",
            "vscode://file/x",
            "ssh://host",
            "HTTPS://example.com",
            "/Applications/Calculator.app",
            "-a Calculator",
            "//example.com",
            "https:example.com",
        ] {
            assert!(check_link(u).is_err(), "{u}");
        }
    }

    #[test]
    fn credentials_odd_hosts_and_odd_characters_are_refused() {
        for u in [
            "https://user:pw@example.com/",
            "https://example.com@evil.example/",
            "https://",
            "https:///path",
            "https://-a/",
            "https://.x/",
            "https://exa mple.com",
            "https://example.com/\n",
            "https://ex\u{e4}mple.com",
            "https://ex_ample.com",
        ] {
            assert!(check_link(u).is_err(), "{u:?}");
        }
        assert!(check_link(&format!("https://e.com/{}", "a".repeat(LINK_MAX))).is_err(), "too long");
    }
}
