use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;

use super::*;
use crate::providers::contract::parse_provider_list;

/// A machine with scripted CLIs: `(name) -> path`, and `(path, args joined) -> (code, stdout)`; a missing answer is a
/// run that did not finish.
#[derive(Default)]
struct Fake {
    bins: HashMap<&'static str, &'static str>,
    answers: HashMap<(&'static str, &'static str), (i32, &'static str)>,
    env: HashMap<&'static str, &'static str>,
}

impl Probe for Fake {
    fn find(&self, name: &str) -> Option<PathBuf> {
        self.bins.get(name).map(PathBuf::from)
    }
    fn run(&self, path: &Path, args: &[&str]) -> Option<Out> {
        let key = (path.to_str().unwrap(), args.join(" "));
        self.answers.iter().find(|((p, a), _)| *p == key.0 && *a == key.1).map(|(_, (c, s))| Out { code: Some(*c), stdout: (*s).into() })
    }
    fn env(&self, key: &str) -> Option<OsString> {
        self.env.get(key).map(OsString::from)
    }
}

fn both() -> Fake {
    let mut f = Fake::default();
    f.bins.insert("claude", "/bin/claude");
    f.bins.insert("codex", "/bin/codex");
    f.answers.insert(("/bin/claude", "--version"), (0, "2.1.296 (Claude Code)\n"));
    f.answers.insert(("/bin/claude", "auth status --json"), (0, r#"{"loggedIn": true, "authMethod": "claude.ai", "email": "someone@example.com"}"#));
    f.answers.insert(("/bin/codex", "--version"), (0, "codex-cli 0.166.0\n"));
    f.answers.insert(("/bin/codex", "login status"), (0, "Logged in using ChatGPT\n"));
    f
}

fn get(l: &ProviderList, id: &str) -> ProviderInfo {
    l.providers.iter().find(|p| p.id == id).unwrap().clone()
}

#[test]
fn g1_an_installed_signed_in_cli_reports_path_version_and_the_clis_own_check() {
    let l = list(&both());
    let c = get(&l, "claude");
    assert_eq!((c.installed, c.path.as_deref(), c.version.as_deref(), c.tested, c.login.state, c.login.checked_by.as_str()), (true, Some("/bin/claude"), Some("2.1.296"), Tested::Tested, LoginState::SignedIn, "cli_status"));
    let x = get(&l, "codex");
    assert_eq!((x.version.as_deref(), x.tested, x.login.state), (Some("0.166.0"), Tested::Tested, LoginState::SignedIn));
    assert!(parse_provider_list(&serde_json::to_vec(&l).unwrap()).is_ok(), "what we answer is what the contract accepts");
}

#[test]
fn g2_a_cli_not_on_the_path_is_not_installed_and_start_and_resume_say_why() {
    let mut f = both();
    f.bins.remove("codex");
    let x = get(&list(&f), "codex");
    assert_eq!((x.installed, x.path.clone(), x.login.state, x.login.checked_by.as_str()), (false, None, LoginState::NotInstalled, "none"));
    assert!(["start", "resume"].iter().all(|c| x.limits.iter().any(|l| l.control == *c && l.reason.contains("not installed"))));
    assert!(parse_provider_list(&serde_json::to_vec(&list(&f)).unwrap()).is_ok());
}

#[test]
fn g3_signed_out_is_reported_for_both_clis() {
    let mut f = both();
    f.answers.insert(("/bin/claude", "auth status --json"), (1, r#"{"loggedIn": false, "authMethod": "none"}"#));
    f.answers.insert(("/bin/codex", "login status"), (1, "Not logged in\n"));
    let l = list(&f);
    assert_eq!((get(&l, "claude").login.state, get(&l, "codex").login.state), (LoginState::SignedOut, LoginState::SignedOut));
}

#[test]
fn g4_a_check_that_cannot_answer_is_cannot_tell_with_a_sentence_and_never_signed_in() {
    let mut f = both();
    // Claude: garbage; Codex: an older CLI with no such command (usage error, exit 2).
    f.answers.insert(("/bin/claude", "auth status --json"), (0, "not json"));
    f.answers.insert(("/bin/codex", "login status"), (2, "error: unrecognized subcommand 'status'"));
    let l = list(&f);
    for id in ["claude", "codex"] {
        let p = get(&l, id);
        assert_eq!(p.login.state, LoginState::CannotTell, "{id}");
        assert!(p.login.detail.is_some(), "{id}");
    }
    // A check that timed out or was cut (no whole answer) is the same.
    f.answers.remove(&("/bin/claude", "auth status --json"));
    assert_eq!(get(&list(&f), "claude").login.state, LoginState::CannotTell);
    // Exit 0 with loggedIn false is signed out, and a loggedIn:true that did not exit 0 is not trusted.
    f.answers.insert(("/bin/claude", "auth status --json"), (3, r#"{"loggedIn": true}"#));
    assert_eq!(get(&list(&f), "claude").login.state, LoginState::CannotTell);
}

#[test]
fn g5_an_untested_version_is_marked_but_still_listed_as_installed() {
    let mut f = both();
    f.answers.insert(("/bin/codex", "--version"), (0, "codex-cli 0.201.0\n"));
    f.answers.insert(("/bin/claude", "--version"), (0, "3.0.0 (Claude Code)\n"));
    let l = list(&f);
    for id in ["claude", "codex"] {
        let p = get(&l, id);
        assert_eq!((p.tested, p.installed), (Tested::Untested, true), "{id}");
    }
    f.answers.remove(&("/bin/claude", "--version"));
    assert_eq!(get(&list(&f), "claude").tested, Tested::Unknown, "a version we could not read");
}

#[test]
fn g6_codex_lists_every_control_it_lacks_with_a_reason() {
    let x = get(&list(&both()), "codex");
    let off: Vec<&str> = x.limits.iter().map(|l| l.control.as_str()).collect();
    for c in ["message", "redirect", "inspect", "transcript", "vajra_workflow"] {
        assert!(off.contains(&c), "{c}: {off:?}");
    }
    assert!(x.limits.iter().filter(|l| l.control != "stop").all(|l| l.reason.starts_with("Codex cannot ")));
    assert!(get(&list(&both()), "claude").limits.is_empty());
}

#[test]
fn g10_a_key_in_the_app_environment_is_named_never_carried() {
    let mut f = both();
    f.env.insert("OPENAI_API_KEY", "sk-test-NOT-A-REAL-KEY");
    let l = list(&f);
    assert_eq!(get(&l, "codex").billing.key_in_app_env.as_deref(), Some("OPENAI_API_KEY"));
    assert_eq!(get(&l, "claude").billing.key_in_app_env, None);
    let text = serde_json::to_string(&l).unwrap();
    assert!(!text.contains("sk-test") && !text.contains("someone@example.com"), "neither a key value nor the status output's account leaves detection");
}

#[test]
fn one_finds_a_provider_by_id_and_nothing_else() {
    assert!(one(&both(), "codex").is_some());
    assert!(one(&both(), "gemini").is_none());
}

#[test]
fn the_real_probe_runs_the_status_command_of_a_script_on_its_path_and_reads_its_answer() {
    let d = std::env::temp_dir().join(format!("vajra-detect-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let bin = d.join("claude");
    std::fs::write(&bin, "#!/bin/sh\ncase \"$1\" in --version) echo '2.0.5 (Claude Code)';; auth) echo '{\"loggedIn\": false}'; exit 1;; esac\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let c = get(&list(&System::new(vec![d.clone()])), "claude");
    assert_eq!((c.installed, c.version.as_deref(), c.login.state), (true, Some("2.0.5"), LoginState::SignedOut));
    assert!(!get(&list(&System::new(vec![d.clone()])), "codex").installed);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn every_limit_and_the_login_carry_a_stable_code_beside_the_text() {
    let mut f = both();
    f.answers.insert(("/bin/codex", "login status"), (2, "usage"));
    let x = get(&list(&f), "codex");
    assert_eq!(x.login.route_code.as_deref(), Some("uses_codex_login"));
    assert_eq!((x.login.detail_code.as_deref(), x.login.state), (Some("check_not_understood"), LoginState::CannotTell));
    assert!(x.limits.iter().all(|l| l.code.is_some()), "{:?}", x.limits);
    let codes: Vec<&str> = x.limits.iter().map(|l| l.code.as_deref().unwrap()).collect();
    for c in ["no_message_mid_run", "no_redirect_mid_run", "no_inspect", "no_transcript", "no_vajra_workflow", "stop_ends_process"] {
        assert!(codes.contains(&c), "{c}: {codes:?}");
    }
    let c = get(&list(&f), "claude");
    assert!(c.limits.is_empty(), "claude stops by a hook hand-off: no stop limit");
    let mut g = both();
    g.bins.remove("codex");
    let nx = get(&list(&g), "codex");
    assert!(nx.limits.iter().any(|l| l.code.as_deref() == Some("not_installed") && l.control == "start"));
}

#[test]
fn an_unsupported_choice_validates_as_a_project_provider() {
    let u = unsupported("gemini");
    assert!(u.validate().is_ok());
    assert!(unsupported("Bad Id").validate().is_err(), "an id that is not a provider id is still refused");
}
