use super::super::tar::Pack;
use super::*;

fn pack(manifest: &str, shrinkwrap: Option<&str>, nested: &[(&str, &str)]) -> Pack {
    Pack {
        manifest: manifest.as_bytes().to_vec(),
        shrinkwrap: shrinkwrap.map(|s| s.as_bytes().to_vec()),
        nested: nested.iter().map(|(d, m)| (d.to_string(), m.as_bytes().to_vec())).collect(),
    }
}

fn set(v: &[(&str, &str)]) -> Closure {
    v.iter().map(|(n, ver)| (n.to_string(), ver.to_string())).collect()
}

const OK_RESOLVED: &str = "https://registry.npmjs.org/dep/-/dep-1.0.0.tgz";

fn wrap(entries: &str) -> String {
    format!(r#"{{"name":"x","version":"1.0.0","lockfileVersion":3,"packages":{{"":{{"name":"x","version":"1.0.0"}}{entries}}}}}"#)
}

#[test]
fn a_package_with_no_dependencies_is_its_own_closure() {
    let p = pack(r#"{"name":"x","version":"1.0.0"}"#, None, &[]);
    assert_eq!(check(&p, "x", "1.0.0"), Ok(set(&[("x", "1.0.0")])));
}

#[test]
fn the_manifest_must_be_the_pinned_package() {
    let p = pack(r#"{"name":"x","version":"1.0.1"}"#, None, &[]);
    assert_eq!(check(&p, "x", "1.0.0"), Err(Refusal::ManifestMismatch));
    assert_eq!(check(&pack(r#"{"name":"y","version":"1.0.0"}"#, None, &[]), "x", "1.0.0"), Err(Refusal::ManifestMismatch));
    assert_eq!(check(&pack("not json", None, &[]), "x", "1.0.0"), Err(Refusal::ManifestMismatch));
}

#[test]
fn a_package_that_needs_its_install_script_is_refused() {
    for s in ["preinstall", "install", "postinstall"] {
        let p = pack(&format!(r#"{{"name":"x","version":"1.0.0","scripts":{{"{s}":"node i.js"}}}}"#), None, &[]);
        assert_eq!(check(&p, "x", "1.0.0"), Err(Refusal::NeedsInstallScripts), "{s}");
    }
    let test_only = pack(r#"{"name":"x","version":"1.0.0","scripts":{"test":"jest"}}"#, None, &[]);
    assert!(check(&test_only, "x", "1.0.0").is_ok());
}

#[test]
fn an_unpinned_dependency_of_any_kind_with_no_shrinkwrap_is_refused() {
    for kind in ["dependencies", "peerDependencies", "optionalDependencies"] {
        let p = pack(&format!(r#"{{"name":"x","version":"1.0.0","{kind}":{{"dep":"^1.0.0"}}}}"#), None, &[]);
        assert_eq!(check(&p, "x", "1.0.0"), Err(Refusal::UnpinnedDependency), "{kind}");
    }
}

#[test]
fn bundled_dependencies_present_in_the_tarball_are_pinned_by_its_hash() {
    let m = r#"{"name":"x","version":"1.0.0","dependencies":{"a":"^1"},"bundleDependencies":["a"]}"#;
    let p = pack(m, None, &[("node_modules/a", r#"{"name":"a","version":"1.2.3"}"#), ("node_modules/a/node_modules/@s/b", r#"{"name":"@s/b","version":"0.1.0"}"#)]);
    assert_eq!(check(&p, "x", "1.0.0"), Ok(set(&[("x", "1.0.0"), ("a", "1.2.3"), ("@s/b", "0.1.0")])));
    let missing = pack(m, None, &[]);
    assert_eq!(check(&missing, "x", "1.0.0"), Err(Refusal::UnpinnedDependency), "named as bundled but not in the tarball");
    let all = pack(r#"{"name":"x","version":"1.0.0","dependencies":{"a":"^1"},"bundledDependencies":true}"#, None, &[("node_modules/a", r#"{"name":"a","version":"1.2.3"}"#)]);
    assert!(check(&all, "x", "1.0.0").is_ok(), "bundledDependencies: true bundles every dependency");
}

#[test]
fn a_complete_shrinkwrap_pins_the_closure() {
    let m = r#"{"name":"x","version":"1.0.0","dependencies":{"dep":"^1.0.0"}}"#;
    let sw = wrap(&format!(r#","node_modules/dep":{{"version":"1.0.0","resolved":"{OK_RESOLVED}","integrity":"sha512-AAAA"}}"#));
    assert_eq!(check(&pack(m, Some(&sw), &[]), "x", "1.0.0"), Ok(set(&[("x", "1.0.0"), ("dep", "1.0.0")])));
}

#[test]
fn a_shrinkwrap_with_a_range_a_missing_integrity_or_another_host_is_refused() {
    let m = r#"{"name":"x","version":"1.0.0","dependencies":{"dep":"^1.0.0"}}"#;
    for entry in [
        format!(r#""version":"^1.0.0","resolved":"{OK_RESOLVED}","integrity":"sha512-AAAA""#),
        format!(r#""version":"1.0.0","resolved":"{OK_RESOLVED}""#),
        format!(r#""version":"1.0.0","resolved":"{OK_RESOLVED}","integrity":"sha1-AAAA""#),
        r#""version":"1.0.0","resolved":"git+https://github.com/a/dep.git","integrity":"sha512-AAAA""#.to_string(),
        r#""version":"1.0.0","resolved":"https://evil.example/dep-1.0.0.tgz","integrity":"sha512-AAAA""#.to_string(),
        r#""version":"1.0.0","resolved":"file:../dep","integrity":"sha512-AAAA""#.to_string(),
        r#""link":true,"resolved":"../dep""#.to_string(),
    ] {
        let sw = wrap(&format!(r#","node_modules/dep":{{{entry}}}"#));
        assert_eq!(check(&pack(m, Some(&sw), &[]), "x", "1.0.0"), Err(Refusal::BadShrinkwrap), "{entry}");
    }
}

#[test]
fn a_shrinkwrap_that_misses_a_direct_dependency_or_has_an_old_lockfile_is_refused() {
    let m = r#"{"name":"x","version":"1.0.0","dependencies":{"dep":"^1.0.0"},"optionalDependencies":{"opt":"1.0.0"}}"#;
    let sw = wrap(&format!(r#","node_modules/dep":{{"version":"1.0.0","resolved":"{OK_RESOLVED}","integrity":"sha512-AAAA"}}"#));
    assert_eq!(check(&pack(m, Some(&sw), &[]), "x", "1.0.0"), Err(Refusal::UnpinnedDependency));
    let v1 = r#"{"name":"x","version":"1.0.0","lockfileVersion":1,"dependencies":{"dep":{"version":"1.0.0"}}}"#;
    assert_eq!(check(&pack(r#"{"name":"x","version":"1.0.0","dependencies":{"dep":"1"}}"#, Some(v1), &[]), "x", "1.0.0"), Err(Refusal::BadShrinkwrap));
}

#[test]
fn closures_compare_by_name_and_version() {
    let a = set(&[("x", "1.0.0"), ("dep", "1.0.0")]);
    assert!(same(&a, &set(&[("dep", "1.0.0"), ("x", "1.0.0")])));
    assert!(!same(&a, &set(&[("x", "1.0.0"), ("dep", "1.0.1")])), "a different version");
    assert!(!same(&a, &set(&[("x", "1.0.0"), ("dep", "1.0.0"), ("evil", "0.0.1")])), "an extra package");
}

#[test]
fn reads_the_installed_tree_from_npm_ls() {
    let ls = r#"{"dependencies":{"x":{"version":"1.0.0","dependencies":{"dep":{"version":"1.0.0","dependencies":{"@s/b":{"version":"0.1.0"}}}}}}}"#;
    assert_eq!(installed(ls, "x"), Ok(Some(set(&[("x", "1.0.0"), ("dep", "1.0.0"), ("@s/b", "0.1.0")]))));
    assert_eq!(installed(r#"{"dependencies":{}}"#, "x"), Ok(None), "not installed");
    assert_eq!(installed("{}", "x"), Ok(None));
    assert_eq!(installed(r#"{"dependencies":{"y":{"version":"1.0.0"}}}"#, "x"), Ok(None), "only another package");
}

#[test]
fn an_npm_ls_that_could_not_be_read_is_a_failure_not_absence() {
    for bad in ["", "not json", "{\"dependencies\":", "[]", "null", "\"x\"", r#"{"dependencies":[]}"#, r#"{"dependencies":{"x":"1.0.0"}}"#] {
        assert_eq!(installed(bad, "x"), Err("npm_ls_failed"), "{bad:?}");
    }
    let error_only = r#"{"error":{"code":"EJSONPARSE","summary":"Unexpected end of JSON input"}}"#;
    assert_eq!(installed(error_only, "x"), Err("npm_ls_failed"), "an error with no package");
    // npm also reports problems (ELSPROBLEMS) next to a tree it did read: the tree counts, and closure checks judge it.
    let with_problems = r#"{"error":{"code":"ELSPROBLEMS"},"dependencies":{"x":{"version":"1.0.0"}}}"#;
    assert_eq!(installed(with_problems, "x"), Ok(Some(set(&[("x", "1.0.0")]))));
}

fn locked(key: &str, extra: &str) -> String {
    format!(r#","{key}":{{"version":"1.0.0","resolved":"{OK_RESOLVED}","integrity":"sha512-AAAA"{extra}}}"#)
}

#[test]
fn a_locked_package_whose_own_dependency_is_not_pinned_is_refused() {
    let m = r#"{"name":"x","version":"1.0.0","dependencies":{"a":"^1.0.0"}}"#;
    for kind in ["dependencies", "peerDependencies", "optionalDependencies"] {
        let sw = wrap(&locked("node_modules/a", &format!(r#","{kind}":{{"b":"^1.0.0"}}"#)));
        assert_eq!(check(&pack(m, Some(&sw), &[]), "x", "1.0.0"), Err(Refusal::UnpinnedDependency), "transitive {kind}");
        let pinned = wrap(&(locked("node_modules/a", &format!(r#","{kind}":{{"b":"^1.0.0"}}"#)) + &locked("node_modules/b", "")));
        assert!(check(&pack(m, Some(&pinned), &[]), "x", "1.0.0").is_ok(), "{kind} pinned at the top resolves by hoisting");
    }
}

#[test]
fn a_dependency_resolves_the_node_way_from_where_it_is_required() {
    let m = r#"{"name":"x","version":"1.0.0","dependencies":{"a":"1","c":"1"}}"#;
    // a's b is nested under a; c also wants b, and can't see a's copy.
    let nested_only = wrap(&(locked("node_modules/a", r#","dependencies":{"b":"1"}"#) + &locked("node_modules/a/node_modules/b", "") + &locked("node_modules/c", r#","dependencies":{"b":"1"}"#)));
    assert_eq!(check(&pack(m, Some(&nested_only), &[]), "x", "1.0.0"), Err(Refusal::UnpinnedDependency));
    let both = wrap(&(locked("node_modules/a", r#","dependencies":{"b":"1"}"#) + &locked("node_modules/a/node_modules/b", "") + &locked("node_modules/c", r#","dependencies":{"b":"1"}"#) + &locked("node_modules/b", "")));
    assert!(check(&pack(m, Some(&both), &[]), "x", "1.0.0").is_ok());
}

#[test]
fn a_bundled_package_with_a_dependency_outside_the_tarball_is_refused() {
    let m = r#"{"name":"x","version":"1.0.0","dependencies":{"a":"^1"},"bundleDependencies":["a"]}"#;
    for kind in ["dependencies", "peerDependencies", "optionalDependencies"] {
        let a = format!(r#"{{"name":"a","version":"1.2.3","{kind}":{{"outside":"^2"}}}}"#);
        assert_eq!(check(&pack(m, None, &[("node_modules/a", &a)]), "x", "1.0.0"), Err(Refusal::UnpinnedDependency), "bundled {kind}");
        let inside = pack(m, None, &[("node_modules/a", &a), ("node_modules/a/node_modules/outside", r#"{"name":"outside","version":"2.0.0"}"#)]);
        assert!(check(&inside, "x", "1.0.0").is_ok(), "{kind} bundled inside a");
    }
}
