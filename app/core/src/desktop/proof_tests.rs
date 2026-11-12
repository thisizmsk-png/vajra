use super::*;

fn done<'a>(slug: &'a str, goal: &'a str, trust: Option<&'a str>) -> Input<'a> {
    Input { slug, goal, status: "done", integrity: "ok", rows: vec!["pass", "pass"], trust, approval_consumed: false, rejected: false, slug_field: slug, ledger: Ledger::Match }
}

#[test]
fn verified_needs_the_ledger_match_and_the_signed_slug_field() {
    let ok = done("deliver-20261112-1900", "g", Some("guarded"));
    assert_eq!(label(&ok).0, Label::Verified);
    // The signature doesn't bind the folder name: a record copied under this slug names another campaign.
    assert_eq!(label(&Input { slug_field: "deliver-20261112-1800", ..ok.clone() }).0, Label::NotVerified);
    assert_eq!(label(&Input { ledger: Ledger::Mismatch, ..ok.clone() }).0, Label::NotVerified);
    assert_eq!(label(&Input { ledger: Ledger::Missing, ..ok.clone() }).0, Label::NotVerified);
    // Outside the guard is not lifted to anything by a ledger either.
    let og = Input { ledger: Ledger::Mismatch, ..done("deliver-20261112-1900-og", "g", None) };
    assert_eq!(label(&og).0, Label::NotVerified);
}

#[test]
fn the_record_note_says_tamper_evident_only_when_switched_on_and_never_for_an_outside_guard_record() {
    for l in [Label::Verified, Label::VerifiedApproved, Label::NotVerified, Label::Rejected, Label::Abandoned, Label::InProgress] {
        assert_eq!(record_note(l), "Checked by Vajra on this Mac. The signing key is guarded, not yet sandboxed.");
    }
    assert!(record_note(Label::EvidenceUnguarded).contains("isn't tamper-evident"));
    assert!(!record_note(Label::EvidenceUnguarded).to_lowercase().starts_with("tamper-evident"));
}

fn proof(label: Label, rows: Vec<(&str, &str)>) -> Proof {
    Proof {
        slug: "deliver-20261112-1900".into(),
        goal: "Add rate limits".into(),
        label,
        reason: "r".into(),
        integrity: "ok".into(),
        record_sha256: Some("ab".repeat(32)),
        rows: rows
            .into_iter()
            .map(|(c, s)| Row { criterion: c.into(), task: "t1".into(), status: s.into(), commit: Some("0123456789abcdef".into()), ran_by: Some("codex (openai)".into()) })
            .collect(),
        report: Some("# Delivery report".into()),
    }
}

#[test]
fn a_guarded_run_whose_criteria_all_pass_is_verified() {
    assert_eq!(label(&done("deliver-20261112-1900", "Add limits", Some("guarded"))).0, Label::Verified);
    // A campaign today's workflow ran (no seats.json) keeps today's trust.
    assert_eq!(label(&done("my-campaign", "Add limits", None)).0, Label::Verified);
}

#[test]
fn outside_the_guard_by_either_source_is_never_plain_verified() {
    // The core's own record says outside, the signed record doesn't.
    assert_eq!(label(&done("deliver-20261112-1900", "Add limits", Some("outside_guard"))).0, Label::EvidenceUnguarded);
    // The signed slug says outside, the core's record says guarded: the worse wins.
    assert_eq!(label(&done("deliver-20261112-1900-og", "Add limits", Some("guarded"))).0, Label::EvidenceUnguarded);
    // The signed goal marker alone.
    let goal = format!("{MARKER} Add limits");
    assert_eq!(label(&done("deliver-20261112-1900", &goal, None)).0, Label::EvidenceUnguarded);
    assert_ne!(Label::EvidenceUnguarded.text(), "Verified");
    assert!(Label::EvidenceUnguarded.text().contains("outside the guard"));
    assert!(Label::EvidenceUnguarded.pass_text().contains("outside the guard"));
}

#[test]
fn an_approval_counts_only_when_this_run_consumed_it() {
    let mut i = done("deliver-20261112-1900", "g", Some("guarded_by_approval"));
    assert_eq!(label(&i).0, Label::NotVerified);
    i.approval_consumed = true;
    assert_eq!(label(&i).0, Label::VerifiedApproved);
    // An approval never lifts an outside-the-guard signed record.
    let mut og = done("deliver-20261112-1900-og", "g", Some("guarded_by_approval"));
    og.approval_consumed = true;
    assert_eq!(label(&og).0, Label::EvidenceUnguarded);
}

#[test]
fn every_failure_ends_not_verified_and_a_bad_record_or_abandoned_run_never_shows_a_pass() {
    let base = done("deliver-20261112-1900", "g", Some("guarded"));
    for (i, want) in [
        (Input { integrity: "integrity_failed", ..base.clone() }, Label::NotVerified),
        (Input { integrity: "unknown", ..base.clone() }, Label::NotVerified),
        (Input { integrity: "no_report", ..base.clone() }, Label::NotVerified),
        (Input { rows: vec!["pass", "stale"], ..base.clone() }, Label::NotVerified),
        (Input { rows: vec!["pass", "unverified"], ..base.clone() }, Label::NotVerified),
        (Input { rows: vec![], ..base.clone() }, Label::NotVerified),
        (Input { status: "failed", ..base.clone() }, Label::NotVerified),
        (Input { trust: Some("?"), ..base.clone() }, Label::NotVerified),
        (Input { status: "abandoned", ..base.clone() }, Label::Abandoned),
        (Input { status: "abandoned", integrity: "integrity_failed", ..base.clone() }, Label::NotVerified),
        (Input { rejected: true, ..base.clone() }, Label::Rejected),
        (Input { status: "active", ..base.clone() }, Label::InProgress),
    ] {
        let (got, why) = label(&i);
        assert_eq!(got, want, "{why}");
        assert!(!why.is_empty());
    }
    assert!(!Label::Abandoned.pass_text().eq("pass"));
}

#[test]
fn the_export_shows_each_label_on_its_matching_fixture() {
    for l in [Label::Verified, Label::VerifiedApproved, Label::EvidenceUnguarded, Label::NotVerified, Label::Rejected, Label::Abandoned] {
        let p = proof(l, vec![("limits apply", "pass"), ("429 body", "stale")]);
        let md = markdown(&p);
        let html = page(&p);
        assert!(md.contains(&format!("**{}**", l.text())), "{l:?}");
        assert!(html.contains(&esc(l.text())), "{l:?}");
        // Exactly the label the app shows: no stronger word anywhere.
        if l != Label::Verified && l != Label::VerifiedApproved {
            assert!(!md.contains("**Verified**"), "{l:?}");
        }
        assert!(md.contains(record_note(l)) && html.contains(&esc(record_note(l))), "{l:?}");
        assert!(md.contains("Checked by Vajra on this Mac"));
        assert!(md.contains(&"ab".repeat(32)), "the record hash is in the export");
        assert!(md.contains("stale evidence"));
        assert!(md.contains("codex (openai)"), "the harness that checked is named");
        for banned in ["verifiable", "cryptographically"] {
            assert!(!md.to_lowercase().contains(banned) && !html.to_lowercase().contains(banned));
        }
    }
    let og = markdown(&proof(Label::EvidenceUnguarded, vec![("c", "pass")]));
    assert!(og.contains(WHY_OUTSIDE));
    assert!(og.contains("verified (evidence ran outside the guard)"));
    let nv = markdown(&proof(Label::NotVerified, vec![("c", "pass")]));
    assert!(nv.contains("**Not verified**") && nv.contains("pass, not verified"));
}

#[test]
fn the_page_is_self_contained_and_escapes_every_value() {
    let mut p = proof(Label::Verified, vec![("<script>alert(1)</script>", "pass")]);
    p.goal = "a \"goal\" <img src=x>".into();
    let html = page(&p);
    assert!(!html.contains("<script>") && !html.contains("<img"));
    assert!(!html.contains("http://") && !html.contains("https://") && !html.contains("<link"));
    assert!(html.contains("default-src 'none'"));
    let md = markdown(&p);
    assert!(!md.contains("<script>") && !md.contains("<img"));
    let mut pipes = proof(Label::Verified, vec![("a | b\nc", "pass")]);
    pipes.goal = "x".into();
    assert!(markdown(&pipes).contains("a \\| b c"));
}

#[test]
fn only_a_plain_github_pull_request_link_is_taken() {
    assert_eq!(parse_pr("https://github.com/acme/web/pull/42"), Some(PrRef { repo: "acme/web".into(), number: 42 }));
    assert_eq!(parse_pr(" https://github.com/acme/web.js/pull/7/ "), Some(PrRef { repo: "acme/web.js".into(), number: 7 }));
    for bad in [
        "http://github.com/acme/web/pull/42",
        "https://github.com/acme/web/issues/42",
        "https://github.com/acme/web/pull/042",
        "https://github.com/acme/web/pull/0",
        "https://github.com/acme/web/pull/42/files",
        "https://github.com/acme/../pull/42",
        "https://github.com/-R/x/pull/1x",
        "https://github.com/acme/web/pull/42?x=1",
        "https://evil.example/acme/web/pull/42",
        "https://github.com/acme/web/pull/9999999999",
        "acme/web#42",
    ] {
        assert_eq!(parse_pr(bad), None, "{bad}");
    }
}

#[test]
fn the_export_has_one_capability_check() {
    assert!(export_allowed());
}
