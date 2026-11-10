use super::*;
use crate::preferences::Agent;
use crate::providers::launch::Refused;

const A: &str = "62278db8-d6e8-48f7-b03e-14d8a8c989f2";
const RUN_A: &str = "20261108-0900";
const PA: &str = "k_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// Defaults from a claude project; refuses every seat whose account is in `refuse`, with that code.
struct Policy {
    refuse: Vec<(&'static str, &'static str)>,
}
impl SeatPolicy for Policy {
    fn agents(&self, _: &str) -> Option<Agents> {
        let mut a = Agents::from_legacy(Agent::Claude);
        a.allowed.push(crate::preferences::Allowed { account: "codex".into(), model: None });
        a.roles.insert("verifier".into(), Choice { account: "codex".into(), model: None, effort: Some("high".into()) });
        Some(a)
    }
    fn check(&self, _: &str, seats: &[SeatCheck]) -> Vec<(String, Refused)> {
        seats.iter().filter_map(|s| self.refuse.iter().find(|(a, _)| *a == s.choice.account).map(|(_, c)| (s.seat.clone(), Refused { code: c, detail: "x".into() }))).collect()
    }
}

fn service(policy: Option<Policy>) -> (Service, PathBuf) {
    let dir = std::env::temp_dir().join(format!("vajra-seats-{}-{}", std::process::id(), nonce::random_hex(4).unwrap()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let mut s = Service::open(&dir, dir.join("audit.log"), None).unwrap();
    s.start_session(A, RUN_A).unwrap();
    s.set_project(A, PA).unwrap();
    s.set_generation(PA, 1).unwrap();
    s.set_seats_enabled(true);
    if let Some(p) = policy {
        s.set_seat_policy(Arc::new(p));
    }
    (s, dir)
}

fn propose(s: &mut Service, args: Value) -> Result<Value> {
    s.call_tool_for_project(PA, 1, "propose_run", &args)
}

fn w(slot: &str, role: &str) -> Value {
    json!({"slot": slot, "role": role, "task": "Do the thing", "account": "claude"})
}

fn open(s: &Service) -> Vec<Proposal> {
    s.open_proposals(Some(PA)).unwrap()
}

fn seat(role: &str, slot: &str) -> WorkerSeat {
    WorkerSeat { slot: slot.into(), role: role.into(), task: "t".into(), account: "claude".into(), model: Some("m/x".into()), effort: Some("high".into()), security: false, network: false, why: "w".into(), source: "dg".into() }
}

fn base_seats() -> Seats {
    Seats { max_in_flight: 4, lead: LeadSeat { account: "claude".into(), model: None, effort: None, source: "role_default".into() }, workers: vec![seat("backend", "w1"), seat("verifier", "w2")] }
}

// ------------------------------------------------------------------------------------------------ the hash

#[test]
fn a_proposal_with_no_workers_keeps_the_v1_hash_byte_for_byte() {
    // Pinned (and checked outside Rust: sha256 of the sorted-key JSON) from the unchanged v1 code: a v1 ledger line written by an older build must still hash like this.
    assert_eq!(proposal_sha("p_x", "add a changelog", "HEAD", 2), "a77017eeaf89c5648c6a264f3bb22961b602971ccaf848f73a969d361a4ccac0", "v1 changed");
}

#[test]
fn every_field_of_the_lead_every_worker_and_max_in_flight_changes_the_hash() {
    let base = base_seats();
    let h = |s: &Seats| proposal_sha_v2("p", "ask", "HEAD", 2, s);
    let h0 = h(&base);
    assert_eq!(h0, h(&base.clone()), "stable");
    let mut seen = vec![h0.clone()];
    let mut vary = |label: &str, f: &dyn Fn(&mut Seats)| {
        let mut s = base.clone();
        f(&mut s);
        let x = h(&s);
        assert!(!seen.contains(&x), "{label} must change the hash");
        seen.push(x);
    };
    vary("max_in_flight", &|s| s.max_in_flight = 5);
    vary("lead.account", &|s| s.lead.account = "codex".into());
    vary("lead.model", &|s| s.lead.model = Some("m/y".into()));
    vary("lead.effort", &|s| s.lead.effort = Some("low".into()));
    vary("lead.source", &|s| s.lead.source = "dg".into());
    for i in 0..2 {
        vary(&format!("w{i}.slot"), &|s| s.workers[i].slot = "w9".into());
        vary(&format!("w{i}.role"), &|s| s.workers[i].role = "docs".into());
        vary(&format!("w{i}.task"), &|s| s.workers[i].task = "other".into());
        vary(&format!("w{i}.account"), &|s| s.workers[i].account = "codex".into());
        vary(&format!("w{i}.model"), &|s| s.workers[i].model = None);
        vary(&format!("w{i}.effort"), &|s| s.workers[i].effort = None);
        vary(&format!("w{i}.security"), &|s| s.workers[i].security = true);
        vary(&format!("w{i}.network"), &|s| s.workers[i].network = true);
        vary(&format!("w{i}.why"), &|s| s.workers[i].why = "no".into());
        vary(&format!("w{i}.source"), &|s| s.workers[i].source = "role_default".into());
    }
    vary("worker count", &|s| s.workers.push(seat("docs", "w3")));
    for (label, sha) in [("project", proposal_sha_v2("q", "ask", "HEAD", 2, &base)), ("ask", proposal_sha_v2("p", "ask2", "HEAD", 2, &base)), ("base", proposal_sha_v2("p", "ask", "main", 2, &base)), ("rounds", proposal_sha_v2("p", "ask", "HEAD", 3, &base))] {
        assert!(!seen.contains(&sha), "{label}");
    }
    assert_ne!(h0, proposal_sha("p", "ask", "HEAD", 2), "v2 is not v1");
}

#[test]
fn nulls_are_written_out_and_the_field_order_is_fixed() {
    let s = base_seats();
    let v = ProposalV2 { schema: "vajra.run_proposal/2", project: "p", ask: "a", base: "HEAD", rounds: 2, max_in_flight: 4, lead: &s.lead, workers: &s.workers[..0] };
    assert_eq!(serde_json::to_string(&v).unwrap(), r#"{"schema":"vajra.run_proposal/2","project":"p","ask":"a","base":"HEAD","rounds":2,"max_in_flight":4,"lead":{"account":"claude","model":null,"effort":null,"source":"role_default"},"workers":[]}"#);
}

#[test]
fn the_stored_proposal_hash_is_v1_without_workers_and_v2_with_them() {
    let (mut s, dir) = service(Some(Policy { refuse: vec![] }));
    propose(&mut s, json!({"ask": "tidy the README"})).unwrap();
    propose(&mut s, json!({"ask": "tidy the README", "workers": [w("w1", "docs")]})).unwrap();
    let o = open(&s);
    assert_eq!((o[0].seats.is_none(), o[0].proposal_sha256.clone()), (true, proposal_sha(PA, "tidy the README", "HEAD", 2)));
    let seats = o[1].seats.clone().unwrap();
    assert_eq!(o[1].proposal_sha256, proposal_sha_v2(PA, "tidy the README", "HEAD", 2, &seats));
    let _ = fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------------------------------------------ arguments

fn invalid(s: &mut Service, args: Value) -> String {
    match propose(s, args) {
        Err(ControlError::InvalidArgs(m)) => m,
        other => panic!("expected invalid_args, got {other:?}"),
    }
}

#[test]
fn a_proposal_with_bad_seat_arguments_is_refused_and_writes_nothing() {
    let (mut s, dir) = service(Some(Policy { refuse: vec![] }));
    let sixteen: Vec<Value> = (1..=17).map(|n| w(&format!("w{n}"), "docs")).collect();
    let mut hidden = w("w1", "docs");
    hidden["task"] = json!("hid\u{200B}den");
    let mut src = w("w1", "docs");
    src["source"] = json!("role_default");
    let mut bad_model = w("w1", "docs");
    bad_model["model"] = json!("sk proj abc");
    let mut bad_effort = w("w1", "docs");
    bad_effort["effort"] = json!("Very High");
    let mut long = w("w1", "docs");
    long["task"] = json!("x".repeat(201));
    let mut extra = w("w1", "docs");
    extra["admin"] = json!(true);
    let cases = [
        ("empty", json!({"ask": "a", "workers": []})),
        ("17 seats", json!({"ask": "a", "workers": sixteen})),
        ("duplicate slot", json!({"ask": "a", "workers": [w("w1", "docs"), w("w1", "tests")]})),
        ("slot w0", json!({"ask": "a", "workers": [w("w0", "docs")]})),
        ("slot w01", json!({"ask": "a", "workers": [w("w01", "docs")]})),
        ("unknown role", json!({"ask": "a", "workers": [w("w1", "architect")]})),
        ("source from the DG", json!({"ask": "a", "workers": [src]})),
        ("lead source", json!({"ask": "a", "lead": {"account": "claude", "source": "dg"}, "workers": [w("w1", "docs")]})),
        ("network on lead", json!({"ask": "a", "lead": {"account": "claude", "network": true}, "workers": [w("w1", "docs")]})),
        ("hidden char", json!({"ask": "a", "workers": [hidden]})),
        ("model", json!({"ask": "a", "workers": [bad_model]})),
        ("effort", json!({"ask": "a", "workers": [bad_effort]})),
        ("long task", json!({"ask": "a", "workers": [long]})),
        ("extra key", json!({"ask": "a", "workers": [extra]})),
        ("max_in_flight 0", json!({"ask": "a", "max_in_flight": 0, "workers": [w("w1", "docs")]})),
        ("max_in_flight 17", json!({"ask": "a", "max_in_flight": 17, "workers": [w("w1", "docs")]})),
        ("lead without workers", json!({"ask": "a", "lead": {"account": "claude"}})),
    ];
    for (label, args) in cases {
        invalid(&mut s, args);
        assert!(open(&s).is_empty(), "{label} wrote a proposal");
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_dg_cannot_send_seats_while_the_core_route_is_off() {
    let (mut s, dir) = service(Some(Policy { refuse: vec![] }));
    s.set_seats_enabled(false);
    for k in ["workers", "lead", "max_in_flight"] {
        let m = invalid(&mut s, json!({"ask": "a", k: 1}));
        assert!(m.contains("unknown property"), "{m}");
    }
    let names = |t: &Value| t.as_array().unwrap().iter().find(|x| x["name"] == "propose_run").unwrap()["inputSchema"]["properties"].as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    assert!(!names(&crate::control::mcp::tools()).iter().any(|k| ["workers", "lead", "max_in_flight"].contains(&k.as_str())));
    let _ = fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------------------------------------------ required seats

#[test]
fn the_verifier_and_reviewer_are_added_from_role_defaults_with_that_source() {
    let (mut s, dir) = service(Some(Policy { refuse: vec![] }));
    propose(&mut s, json!({"ask": "tidy the README", "workers": [w("w1", "docs")]})).unwrap();
    let seats = open(&s)[0].seats.clone().unwrap();
    let v = seats.first_with_role("verifier").unwrap();
    let r = seats.first_with_role("reviewer").unwrap();
    assert_eq!((v.source.as_str(), v.account.as_str(), v.effort.as_deref(), v.slot.as_str()), ("role_default", "codex", Some("high"), "w2"), "the verifier role has its own default");
    assert_eq!((r.source.as_str(), r.account.as_str(), r.slot.as_str()), ("role_default", "claude", "w3"));
    assert!(seats.first_with_role("security_reviewer").is_none(), "no security words, no security reviewer");
    assert_eq!((seats.workers[0].source.as_str(), seats.lead.source.as_str(), seats.max_in_flight), ("dg", "role_default", 4));
    // A proposed verifier is kept as proposed and not duplicated.
    propose(&mut s, json!({"ask": "tidy the README", "workers": [w("w1", "verifier")]})).unwrap();
    let seats = open(&s)[1].seats.clone().unwrap();
    assert_eq!(seats.workers.iter().filter(|x| x.role == "verifier").count(), 1);
    assert_eq!(seats.first_with_role("verifier").unwrap().source, "dg");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_ask_that_names_security_work_adds_a_security_reviewer_and_prefers_a_marked_one() {
    let (mut s, dir) = service(Some(Policy { refuse: vec![] }));
    propose(&mut s, json!({"ask": "rotate the session tokens", "workers": [w("w1", "backend")]})).unwrap();
    let seats = open(&s)[0].seats.clone().unwrap();
    assert_eq!(seats.first_with_role("security_reviewer").map(|x| x.source.as_str()), Some("role_default"));
    // Proposed ones are used as sent; the core adds none when one is there, marked or not.
    let mut marked = w("w2", "security_reviewer");
    marked["security"] = json!(true);
    propose(&mut s, json!({"ask": "rotate the session tokens", "workers": [w("w1", "security_reviewer"), marked]})).unwrap();
    let seats = open(&s)[1].seats.clone().unwrap();
    assert_eq!(seats.workers.iter().filter(|x| x.role == "security_reviewer").count(), 2);
    assert!(seats.workers.iter().all(|x| x.source == "dg" || x.role != "security_reviewer"));
    // Never refused or held for lack of a marked seat (N15).
    assert!(open(&s).len() == 2);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_verifier_with_no_default_is_refused() {
    let (mut s, dir) = service(None);
    let m = invalid(&mut s, json!({"ask": "a", "lead": {"account": "claude"}, "workers": [w("w1", "docs")]}));
    assert!(m.contains("verifier"), "{m}");
    assert!(open(&s).is_empty());
    // With every required seat sent, no default is needed.
    propose(&mut s, json!({"ask": "a", "lead": {"account": "claude"}, "workers": [w("w1", "verifier"), w("w2", "reviewer")]})).unwrap();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_full_roster_cannot_take_the_required_seat() {
    let (mut s, dir) = service(Some(Policy { refuse: vec![] }));
    let sixteen: Vec<Value> = (1..=16).map(|n| w(&format!("w{n}"), "docs")).collect();
    let m = invalid(&mut s, json!({"ask": "a", "workers": sixteen}));
    assert!(m.contains("free slot"), "{m}");
    let _ = fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------------------------------------------ refusals

#[test]
fn every_failing_seat_is_listed_in_one_answer_and_nothing_is_written() {
    let (mut s, dir) = service(Some(Policy { refuse: vec![("codex", "key_missing"), ("openrouter", "harness_unsupported")] }));
    let mut a = w("w1", "docs");
    a["account"] = json!("openrouter");
    let mut b = w("w2", "tests");
    b["account"] = json!("codex");
    let r = propose(&mut s, json!({"ask": "a", "lead": {"account": "codex"}, "workers": [a, b, w("w3", "backend")]}));
    let Err(ControlError::Refusals(list)) = r else { panic!("{r:?}") };
    let got: Vec<(String, String)> = list.iter().map(|x| (x["seat"].as_str().unwrap().into(), x["code"].as_str().unwrap().into())).collect();
    // The default verifier seat (the role default is codex here) fails too, and the lead.
    assert!(got.contains(&("lead".into(), "key_missing".into())) && got.contains(&("w1".into(), "harness_unsupported".into())) && got.contains(&("w2".into(), "key_missing".into())), "{got:?}");
    assert!(got.iter().any(|(seat, _)| seat == "w4"), "the added verifier is checked too: {got:?}");
    assert!(!got.iter().any(|(seat, _)| seat == "w3"));
    assert_eq!(ControlError::Refusals(list).code(), "invalid_args");
    assert!(open(&s).is_empty(), "no proposal line");
    assert!(!fs::read_to_string(dir.join("ledger.jsonl")).unwrap_or_default().contains("\"propose\""));
    let _ = fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------------------------------------------ the card

#[test]
fn the_security_warning_shows_when_the_ask_names_security_work_and_no_seat_is_marked() {
    let (mut s, dir) = service(Some(Policy { refuse: vec![] }));
    propose(&mut s, json!({"ask": "fix the login redirect", "workers": [w("w1", "backend")]})).unwrap();
    let mut marked = w("w1", "backend");
    marked["security"] = json!(true);
    propose(&mut s, json!({"ask": "fix the login redirect", "workers": [marked]})).unwrap();
    propose(&mut s, json!({"ask": "tidy the README", "workers": [w("w1", "docs")]})).unwrap();
    let mut task = w("w1", "backend");
    task["task"] = json!("Harden the webhook handler");
    propose(&mut s, json!({"ask": "tidy the README", "workers": [task]})).unwrap();
    let o = open(&s);
    let got: Vec<Option<&str>> = o.iter().map(|p| p.security_warning.as_deref()).collect();
    assert_eq!(got, vec![Some(SECURITY_WARNING), None, None, Some(SECURITY_WARNING)]);
    let _ = fs::remove_dir_all(&dir);
}
