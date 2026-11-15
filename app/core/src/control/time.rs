//! UTC timestamps without a date crate: `YYYY-MM-DDTHH:MM:SSZ` (what the hook
//! compares `expires` against) and a millisecond form for the ledger.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn unix(t: SystemTime) -> Duration {
    t.duration_since(UNIX_EPOCH).unwrap_or_default()
}

pub fn unix_ms(t: SystemTime) -> u64 {
    unix(t).as_millis() as u64
}

// Howard Hinnant's civil-from-days and days-from-civil.
pub(crate) fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn days(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn parts(t: SystemTime) -> (String, u32) {
    let d = unix(t);
    let secs = d.as_secs() as i64;
    let (y, mo, da) = civil(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    (format!("{y:04}-{mo:02}-{da:02}T{:02}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60), d.subsec_millis())
}

/// `2026-11-05T10:00:00Z`
pub fn iso_secs(t: SystemTime) -> String {
    format!("{}Z", parts(t).0)
}

/// `2026-11-05T10:00:00.123Z`
pub fn iso_ms(t: SystemTime) -> String {
    let (s, ms) = parts(t);
    format!("{s}.{ms:03}Z")
}

/// Unix seconds of `YYYY-MM-DDTHH:MM:SS[.fff]Z`; `None` for anything else.
pub fn parse_iso(s: &str) -> Option<i64> {
    let s = s.strip_suffix('Z')?;
    let s = match s.split_once('.') {
        Some((a, frac)) if !frac.is_empty() && frac.bytes().all(|b| b.is_ascii_digit()) => a,
        Some(_) => return None,
        None => s,
    };
    let b = s.as_bytes();
    if b.len() != 19 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    // `parse::<u32>` would accept a leading `+`; every other byte must be an ASCII digit.
    if !b.iter().enumerate().all(|(i, c)| matches!(i, 4 | 7 | 10 | 13 | 16) || c.is_ascii_digit()) {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<u32>().ok();
    let (y, mo, d, h, mi, se) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    Some(days(i64::from(y), mo, d) * 86_400 + i64::from(h * 3600 + mi * 60 + se))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_and_parses_round_trip() {
        let t = UNIX_EPOCH + Duration::from_millis(1_793_872_800_123);
        assert_eq!(iso_secs(t), "2026-11-05T10:00:00Z");
        assert_eq!(iso_ms(t), "2026-11-05T10:00:00.123Z");
        assert_eq!(parse_iso("2026-11-05T10:00:00Z"), Some(1_793_872_800));
        assert_eq!(parse_iso("2026-11-05T10:00:00.5Z"), Some(1_793_872_800));
        assert_eq!(parse_iso("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso("2024-02-29T23:59:59Z"), Some(1_709_251_199));
        for bad in ["", "2026-11-05 10:00:00Z", "2026-11-05T10:00:00", "2026-13-05T10:00:00Z", "2026-11-05T10:00:00.xZ", "+026-11-05T10:00:00Z", "2026-11-05T+0:00:00Z"] {
            assert_eq!(parse_iso(bad), None, "{bad}");
        }
    }
}
