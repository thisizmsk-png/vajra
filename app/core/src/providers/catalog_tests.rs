use super::*;

fn bundled() -> &'static Catalog {
    catalog().expect("the compiled-in catalog parses")
}

#[test]
fn the_compiled_in_catalog_parses_and_every_entry_has_a_pin_and_a_source_url() {
    let c = bundled();
    assert!(!c.harnesses.is_empty());
    for h in &c.harnesses {
        assert!(h.docs_url.starts_with("https://"), "{}: docs_url {}", h.id, h.docs_url);
        assert!(h.methods.iter().any(Method::is_pinned), "{}: no pinned method", h.id);
    }
    assert!(validate(c).is_ok(), "{:?}", validate(c));
}

#[test]
fn the_launch_entries_are_the_three_confirmed_clis_and_nothing_is_guessed() {
    let ids: Vec<&str> = bundled().harnesses.iter().map(|h| h.id.as_str()).collect();
    assert_eq!(ids, ["claude", "codex", "gemini"]);
}

#[test]
fn every_launch_entry_ships_unverified_so_none_is_one_click() {
    for h in &bundled().harnesses {
        assert_eq!(h.source_verified_by, None, "{} must wait for the security review", h.id);
        assert!(!h.verified(), "{}", h.id);
    }
}

#[test]
fn the_sign_in_argv_starts_with_the_entry_binary() {
    for h in &bundled().harnesses {
        assert_eq!(h.sign_in.argv.first(), Some(&h.binary), "{}", h.id);
    }
}

#[test]
fn sign_in_runs_the_login_subcommand_where_the_vendor_has_one() {
    // A bare `codex` starts a full agent session, not a login. Gemini CLI has no login subcommand, so it stays bare.
    let argv = |id: &str| find(id).expect(id).sign_in.argv.clone();
    assert_eq!(argv("claude"), ["claude", "auth", "login"]);
    assert_eq!(argv("codex"), ["codex", "login"]);
    assert_eq!(argv("gemini"), ["gemini"]);
}

fn parse(text: &str) -> Result<Catalog, CatalogError> {
    let c: Catalog = serde_json::from_str(text).map_err(|e| CatalogError(e.to_string()))?;
    validate(&c).map(|_| c)
}

fn entry(methods: &str, extra: &str) -> String {
    format!(
        r#"{{"schema":"vajra.harness_catalog/1","harnesses":[{{"id":"x","name":"X","binary":"x","detect_args":["--version"],"adapter":null,
        "sign_in":{{"argv":["x","login"]}},"docs_url":"https://example.com/x","source_verified_by":null,"source_verified_on":null,
        "verification_ref":null,"allowed_binaries":["x"],"methods":[{methods}]{extra}}}],
        "engine_scripts":{{"manual_command":"bash install.sh"}}}}"#
    )
}

const NPM: &str = r#"{"kind":"npm","package":"x","version":"1.2.3","integrity":"sha512-AAAA","command":"npm install -g x"}"#;

#[test]
fn a_well_formed_entry_parses() {
    assert!(parse(&entry(NPM, "")).is_ok(), "{:?}", parse(&entry(NPM, "")));
}

#[test]
fn an_unknown_field_is_refused() {
    assert!(parse(&entry(NPM, r#","install_now":true"#)).is_err());
}

#[test]
fn an_npm_method_needs_an_exact_version_and_a_sha512_integrity() {
    for bad in [
        r#"{"kind":"npm","package":"x","version":"^1.2.3","integrity":"sha512-AAAA","command":"c"}"#,
        r#"{"kind":"npm","package":"x","version":"latest","integrity":"sha512-AAAA","command":"c"}"#,
        r#"{"kind":"npm","package":"x","version":"1.2.3","integrity":"sha1-AAAA","command":"c"}"#,
        r#"{"kind":"npm","package":"x","version":"1.2.3","command":"c"}"#,
    ] {
        assert!(parse(&entry(bad, "")).is_err(), "{bad}");
    }
}

#[test]
fn a_brew_method_names_only_an_official_tap() {
    let ok = r#"{"kind":"brew_cask","package":"x","tap":"homebrew/cask","command":"brew install --cask x"}"#;
    assert!(parse(&entry(ok, "")).is_ok());
    let formula = r#"{"kind":"brew_formula","package":"x","tap":"homebrew/core","command":"brew install x"}"#;
    assert!(parse(&entry(formula, "")).is_ok());
    for bad in [
        r#"{"kind":"brew_cask","package":"x","tap":"someone/tap","command":"c"}"#,
        r#"{"kind":"brew_cask","package":"x","tap":"homebrew/core","command":"c"}"#,
        r#"{"kind":"brew_formula","package":"x","tap":"homebrew/cask","command":"c"}"#,
        r#"{"kind":"brew_formula","package":"x","command":"c"}"#,
    ] {
        assert!(parse(&entry(bad, "")).is_err(), "{bad}");
    }
}

#[test]
fn a_vendor_script_needs_an_https_url_and_a_sha256() {
    let ok = format!(r#"{{"kind":"vendor_script","url":"https://example.com/i.sh","sha256":"{}","self_verifying":false,"command":"c"}}"#, "a".repeat(64));
    assert!(parse(&entry(&ok, "")).is_ok());
    for bad in [
        format!(r#"{{"kind":"vendor_script","url":"http://example.com/i.sh","sha256":"{}","self_verifying":false,"command":"c"}}"#, "a".repeat(64)),
        r#"{"kind":"vendor_script","url":"https://example.com/i.sh","sha256":"abc","self_verifying":false,"command":"c"}"#.to_string(),
    ] {
        assert!(parse(&entry(&bad, "")).is_err(), "{bad}");
    }
}

#[test]
fn ids_are_unique() {
    let mut v: serde_json::Value = serde_json::from_str(&entry(NPM, "")).unwrap();
    let h = v["harnesses"][0].clone();
    v["harnesses"].as_array_mut().unwrap().push(h);
    assert!(parse(&v.to_string()).is_err());
}

#[test]
fn the_engine_ships_unverified_until_the_security_review_signs_off() {
    assert!(!bundled().engine_scripts.verified(), "the engine install stays off until bhishma signs off the D7 PR");
}

#[test]
fn the_engine_verified_fields_are_all_or_none() {
    let with = |fields: &str| entry(NPM, "").replace(r#""manual_command":"bash install.sh""#, &format!(r#""manual_command":"bash install.sh",{fields}"#));
    let by = r#""source_verified_by":"bhishma""#;
    let on = r#""source_verified_on":"2026-11-12""#;
    let r = r#""verification_ref":"https://github.com/o/r/pull/1""#;
    for partial in [by.to_string(), format!("{by},{on}"), format!("{on},{r}"), r.to_string()] {
        assert!(parse(&with(&partial)).is_err(), "{partial}");
    }
    let full = parse(&with(&format!("{by},{on},{r}"))).expect("all three set");
    assert!(full.engine_scripts.verified());
    assert!(parse(&with(&format!(r#"{by},"source_verified_on":"12-11-2026",{r}"#))).is_err(), "a date is YYYY-MM-DD");
    assert!(parse(&with(&format!(r#"{by},{on},"verification_ref":"http://x/1""#))).is_err(), "the ref is https");
    assert!(!parse(&entry(NPM, "")).unwrap().engine_scripts.verified(), "absent fields read as unverified");
}

#[test]
fn a_harness_may_not_take_the_engine_id() {
    let e = entry(NPM, "").replace(r#""id":"x""#, r#""id":"engine""#);
    assert!(parse(&e).unwrap_err().0.contains("reserved"), "{:?}", parse(&e));
}

#[test]
fn a_docs_url_must_be_https() {
    let e = entry(NPM, "").replace("https://example.com/x", "http://example.com/x");
    assert!(parse(&e).is_err());
}

#[test]
fn the_sign_in_argv_must_start_with_the_binary_and_hold_no_shell_text() {
    for argv in [r#"["y","login"]"#, r#"["x","login; rm -rf ~"]"#, r#"["x","$(id)"]"#, r#"[]"#] {
        let e = entry(NPM, "").replace(r#"["x","login"]"#, argv);
        assert!(parse(&e).is_err(), "{argv}");
    }
}

#[test]
fn a_verified_entry_needs_a_date_and_a_review_reference() {
    let only_by = entry(NPM, "").replace(r#""source_verified_by":null"#, r#""source_verified_by":"bhishma""#);
    assert!(parse(&only_by).is_err());
    let full = only_by.replace(r#""source_verified_on":null"#, r#""source_verified_on":"2026-11-11""#).replace(r#""verification_ref":null"#, r#""verification_ref":"https://github.com/o/r/pull/1""#);
    assert!(parse(&full).is_ok(), "{:?}", parse(&full));
    assert!(parse(&full).unwrap().harnesses[0].verified());
}

#[test]
fn the_binary_must_be_a_plain_name_listed_in_allowed_binaries() {
    for bin in ["../x", "/usr/bin/x", "x y"] {
        let e = entry(NPM, "").replace(r#""binary":"x""#, &format!(r#""binary":"{bin}""#));
        assert!(parse(&e).is_err(), "{bin}");
    }
    let unlisted = entry(NPM, "").replace(r#""allowed_binaries":["x"]"#, r#""allowed_binaries":["node"]"#);
    assert!(parse(&unlisted).is_err());
}
