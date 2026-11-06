//! The ids the app makes and accepts: session uuids, run ids, and the clock
//! minute a run id names. One rule each, used at every boundary.

use crate::control::{nonce, time};

/// A canonical 8-4-4-4-12 hex uuid: what Claude Code and Codex name a session.
/// It never starts with `-`, so it cannot read as an option in an argv, and it
/// is a valid control dir name for the hook (`^[A-Za-z0-9-]{8,128}$`).
pub fn valid_session_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}

/// A v4 uuid from the one randomness source (`nonce::SOURCE`).
pub fn uuid_v4() -> std::io::Result<String> {
    let mut b = nonce::random_hex(16)?.into_bytes();
    b[12] = b'4';
    b[16] = b"89ab"[(b[16] as char).to_digit(16).unwrap_or(0) as usize & 0x3];
    let s = String::from_utf8(b).expect("hex is ascii");
    Ok(format!("{}-{}-{}-{}-{}", &s[0..8], &s[8..12], &s[12..16], &s[16..20], &s[20..32]))
}

/// `YYYYMMDD-HHMM`.
pub fn valid_run_id(r: &str) -> bool {
    let b = r.as_bytes();
    b.len() == 13 && b[8] == b'-' && b.iter().enumerate().all(|(i, c)| i == 8 || c.is_ascii_digit())
}

/// UTC `YYYYMMDD-HHMM` for a unix time.
pub fn run_id_at(unix_secs: u64) -> String {
    let (y, m, d) = time::civil((unix_secs / 86_400) as i64);
    let rem = unix_secs % 86_400;
    format!("{y:04}{m:02}{d:02}-{:02}{:02}", rem / 3600, rem % 3600 / 60)
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_are_canonical_uuids_only() {
        assert!(valid_session_id("1b4e28ba-2fa1-41d2-883f-0016d3cca427"));
        for bad in ["", "--last", "../x", "sess-1234abcd", "1b4e28ba-2fa1-41d2-883f-0016d3cca42Z", "1b4e28ba2fa141d2883f0016d3cca427"] {
            assert!(!valid_session_id(bad), "{bad}");
        }
    }

    #[test]
    fn uuid_v4_is_v4_shaped_and_accepted() {
        for _ in 0..64 {
            let u = uuid_v4().unwrap();
            assert!(valid_session_id(&u), "{u}");
            assert_eq!(&u[14..15], "4");
            assert!("89ab".contains(&u[19..20]), "{u}");
        }
    }

    #[test]
    fn run_ids() {
        assert_eq!(run_id_at(0), "19700101-0000");
        assert_eq!(run_id_at(951_782_400), "20000229-0000");
        assert_eq!(run_id_at(1_798_761_599), "20261231-2359");
        assert!(valid_run_id("20261105-1230"));
        assert!(!valid_run_id("2026110-51230") && !valid_run_id("20261105_1230") && !valid_run_id(""));
    }
}
