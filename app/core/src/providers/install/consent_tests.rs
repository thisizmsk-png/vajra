use std::time::{Duration, Instant};

use super::*;

const H: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const H2: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn a_token_is_128_random_bits_and_works_once_for_its_id_and_plan() {
    let c = Consents::default();
    let now = Instant::now();
    let t = c.mint("codex", H, now);
    assert_eq!(t.len(), 32);
    assert!(t.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_ne!(t, c.mint("codex", H, now), "each mint is fresh");
    assert_eq!(c.take(&t, "codex", H, now), Ok(()));
    assert_eq!(c.take(&t, "codex", H, now), Err(ConsentError::Required), "used once");
}

#[test]
fn a_token_for_another_plan_or_harness_fails_and_is_spent() {
    let c = Consents::default();
    let now = Instant::now();
    let t = c.mint("codex", H, now);
    assert_eq!(c.take(&t, "codex", H2, now), Err(ConsentError::Required), "the plan changed");
    assert_eq!(c.take(&t, "codex", H, now), Err(ConsentError::Required), "a failed try still spends it");
    let t = c.mint("codex", H, now);
    assert_eq!(c.take(&t, "claude", H, now), Err(ConsentError::Required), "another harness");
    assert_eq!(c.take("not-a-token", "codex", H, now), Err(ConsentError::Required));
}

#[test]
fn a_token_expires_after_two_minutes() {
    let c = Consents::default();
    let now = Instant::now();
    let t = c.mint("codex", H, now);
    assert_eq!(c.take(&t, "codex", H, now + TTL + Duration::from_secs(1)), Err(ConsentError::Required));
}

#[test]
fn at_most_four_tokens_are_outstanding() {
    let c = Consents::default();
    let now = Instant::now();
    let first = c.mint("codex", H, now);
    let rest: Vec<String> = (1..=MAX).map(|i| c.mint("codex", H, now + Duration::from_millis(i as u64))).collect();
    assert_eq!(c.take(&first, "codex", H, now), Err(ConsentError::Required), "the oldest made room");
    assert_eq!(c.take(&rest[0], "codex", H, now), Ok(()));
}

#[test]
fn one_run_at_a_time_per_harness() {
    let c = Consents::default();
    let g = c.begin("codex").unwrap();
    assert_eq!(c.begin("codex").err(), Some(ConsentError::Busy));
    assert!(c.begin("claude").is_ok(), "another harness may run");
    drop(g);
    assert!(c.begin("codex").is_ok(), "free again once the run ends");
}

#[test]
fn a_token_never_appears_in_debug_output() {
    let c = Consents::default();
    let t = c.mint("codex", H, Instant::now());
    let shown = format!("{c:?}");
    assert!(!shown.contains(&t), "{shown}");
    assert!(shown.contains("outstanding: 1"), "{shown}");
}
