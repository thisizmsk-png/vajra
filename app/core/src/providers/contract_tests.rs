use std::path::PathBuf;

use super::*;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/contract").join(name)).unwrap()
}

#[test]
fn the_documented_list_parses_with_honest_limits_and_a_key_name_only() {
    let l = parse_provider_list(&fixture("providers.v1.json")).unwrap();
    let (c, x) = (&l.providers[0], &l.providers[1]);
    assert_eq!((c.id.as_str(), c.installed, c.tested, c.login.state, c.capabilities.stop), ("claude", true, Tested::Tested, LoginState::SignedIn, StopKind::HookBlock));
    assert!(c.limits.is_empty());
    assert_eq!((x.id.as_str(), x.tested, x.login.state, x.login.checked_by.as_str()), ("codex", Tested::Untested, LoginState::CannotTell, "none"));
    assert_eq!(x.billing.key_in_app_env.as_deref(), Some("OPENAI_API_KEY"), "a name, never a value");
    assert!(!x.capabilities.message && x.limits.iter().any(|l| l.control == "message"), "an off control carries its reason");
}

#[test]
fn a_provider_that_is_not_installed_has_no_path_and_says_so() {
    let l = parse_provider_list(&fixture("providers.v1.not-installed.json")).unwrap();
    let p = &l.providers[0];
    assert_eq!((p.installed, p.path.clone(), p.login.state), (false, None, LoginState::NotInstalled));
}

#[test]
fn unknown_states_read_as_the_cautious_value_and_unknown_fields_are_ignored() {
    let l = parse_provider_list(&fixture("providers.v1.unknown-enums.json")).unwrap();
    let p = &l.providers[0];
    assert_eq!((p.login.state, p.tested, p.capabilities.stop), (LoginState::CannotTell, Tested::Unknown, StopKind::Other), "an unrecognised login state is never signed_in");
}

#[test]
fn another_major_is_reported_not_misread() {
    assert_eq!(parse_provider_list(&fixture("providers.v2.json")), Err(ContractError::UnsupportedVersion { found: "vajra.providers/2".into(), supported: 1 }));
}

#[test]
fn the_project_state_provider_object_parses() {
    let p = parse_project_provider(&fixture("project_state.provider.v1.json")).unwrap();
    assert_eq!((p.id.as_str(), p.installed, p.login_state), ("codex", true, LoginState::CannotTell));
    assert_eq!(p.limits.len(), 3);
}

#[test]
fn a_list_round_trips_and_an_oversized_answer_is_refused_unread() {
    let l = parse_provider_list(&fixture("providers.v1.json")).unwrap();
    assert_eq!(parse_provider_list(&serde_json::to_vec(&l).unwrap()).unwrap(), l);
    assert_eq!(parse_provider_list(&vec![b' '; MAX_BYTES + 1]), Err(ContractError::TooLarge));
}

fn patch(doc: &mut serde_json::Value, case: &serde_json::Value) {
    let mut cur = doc;
    for part in case["at"].as_str().unwrap().split('.').filter(|p| !p.is_empty()) {
        cur = match part.parse::<usize>() {
            Ok(i) => &mut cur[i],
            Err(_) => &mut cur[part],
        };
    }
    for (k, v) in case.get("set").and_then(|s| s.as_object()).into_iter().flatten() {
        cur[k] = v.clone();
    }
    for k in case.get("unset").and_then(|u| u.as_array()).into_iter().flatten() {
        cur.as_object_mut().unwrap().remove(k.as_str().unwrap());
    }
}

fn reason(e: &ContractError) -> &'static str {
    match e {
        ContractError::Malformed(_) => "malformed",
        ContractError::Invalid(_) => "invalid",
        ContractError::UnsupportedVersion { .. } => "unsupported_version",
        ContractError::TooLarge => "too_large",
    }
}

#[test]
fn every_shared_invalid_case_is_rejected_for_its_reason() {
    let cases: serde_json::Value = serde_json::from_slice(&fixture("providers.invalid.v1.json")).unwrap();
    let mut seen = 0;
    for (kind, file) in [("providers", "providers.v1.json"), ("project_provider", "project_state.provider.v1.json")] {
        for c in cases[kind].as_array().unwrap() {
            let mut doc: serde_json::Value = serde_json::from_slice(&fixture(file)).unwrap();
            patch(&mut doc, c);
            let bytes = serde_json::to_vec(&doc).unwrap();
            let got = if kind == "providers" { parse_provider_list(&bytes).map(|_| ()) } else { parse_project_provider(&bytes).map(|_| ()) };
            let e = got.expect_err(&format!("{kind}: {} was accepted", c["case"]));
            assert_eq!(reason(&e), c["reason"].as_str().unwrap(), "{kind}: {} -> {e:?}", c["case"]);
            seen += 1;
        }
    }
    assert!(seen >= 30, "{seen}");
}

#[test]
fn codes_ride_beside_the_fallback_text_and_stop_gets_its_own_limit() {
    let l = parse_provider_list(&fixture("providers.v1.json")).unwrap();
    let x = &l.providers[1];
    assert_eq!(x.login.route_code.as_deref(), Some("uses_codex_login"));
    assert_eq!(x.login.detail_code.as_deref(), Some("check_not_understood"));
    let stop = x.limits.iter().find(|l| l.control == "stop").unwrap();
    assert_eq!((stop.code.as_deref(), stop.reason.as_str()), (Some("stop_ends_process"), "Codex has no hand-off: Stop ends its process"));
    // A record from before codes existed still parses, with no code.
    let old = parse_provider_list(&fixture("providers.v1.not-installed.json")).unwrap();
    assert!(old.providers[0].login.route_code.is_none() && old.providers[0].limits[0].code.is_none());
}

#[test]
fn a_chosen_provider_this_build_does_not_know_is_present_and_cannot_start() {
    let p = parse_project_provider(&fixture("project_state.provider.unsupported.v1.json")).unwrap();
    assert_eq!((p.id.as_str(), p.installed, p.login_state), ("gemini", false, LoginState::NotInstalled));
    assert!(p.limits.iter().all(|l| l.code.as_deref() == Some("not_supported")) && p.limits.iter().any(|l| l.control == "start"));
    assert_eq!(p, crate::providers::detect::unsupported("gemini"), "the core answers exactly this");
}

// ---- accounts ----------------------------------------------------------------------------------------------------------

fn acct(extra: &str) -> String {
    format!(r#"{{"schema":"vajra.provider_accounts/1","accounts":[{{"id":"openrouter","provider":"openrouter","harness":"codex","credential":{{"source":"api_key","saved":true}},"billing":{{"kind":"gateway","account_label":"OpenRouter key"}},"label":"OpenRouter","state":"key_saved","models":[{{"id":"x-ai/grok-4","family":"grok"}}]{extra}}}]}}"#)
}

#[test]
fn a_gateway_account_parses_and_round_trips_with_only_a_saved_flag() {
    let l = parse_account_list(acct("").as_bytes()).unwrap();
    let a = &l.accounts[0];
    assert_eq!((a.credential.source, a.credential.saved, a.state, a.billing.kind), (CredentialSource::ApiKey, Some(true), AccountState::KeySaved, BillingKind::Gateway));
    assert_eq!(parse_account_list(&serde_json::to_vec(&l).unwrap()).unwrap(), l);
}

#[test]
fn account_unknowns_read_cautiously_and_unknown_fields_are_ignored() {
    let raw = acct(r#","future":1"#).replace("key_saved", "expired").replace("\"gateway\"", "\"barter\"");
    let a = &parse_account_list(raw.as_bytes()).unwrap().accounts[0];
    assert_eq!((a.state, a.billing.kind), (AccountState::CannotTell, BillingKind::Unknown), "never signed in, never a guess");
}

#[test]
fn another_accounts_major_is_reported_and_bad_records_are_refused() {
    let v2 = acct("").replace("provider_accounts/1", "provider_accounts/2");
    assert!(matches!(parse_account_list(v2.as_bytes()), Err(ContractError::UnsupportedVersion { supported: 1, .. })));
    let cli_with_saved = acct("").replace("\"api_key\"", "\"cli_login\"");
    assert_eq!(parse_account_list(cli_with_saved.as_bytes()), Err(ContractError::Invalid("credential")));
    let key_without_flag = acct("").replace(",\"saved\":true", "");
    assert_eq!(parse_account_list(key_without_flag.as_bytes()), Err(ContractError::Invalid("credential")));
    let warn = acct("").replace("\"account_label\":\"OpenRouter key\"", "\"override_warning\":{\"name\":\"sk-abc 123\",\"found_in\":\"cli_config\"}");
    assert_eq!(parse_account_list(warn.as_bytes()), Err(ContractError::Invalid("override_warning")), "a value-shaped name is refused");
    let one = acct("").split_once("\"accounts\":[").unwrap().1.strip_suffix("]}").unwrap().to_string();
    let two = format!(r#"{{"schema":"vajra.provider_accounts/1","accounts":[{one},{one}]}}"#);
    assert_eq!(parse_account_list(two.as_bytes()), Err(ContractError::Invalid("account id")));
}
