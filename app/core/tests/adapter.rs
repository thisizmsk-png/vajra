//! Agent adapters: honest capability flags, argv for the stock CLIs, and a
//! billing source that never touches a credential's value.

use std::ffi::OsString;

use vajra_desktop::adapter::{adapter_for, Billing, Engine, StopKind};
use vajra_desktop::sessions::argv::RunInput;

fn input(ask: &str) -> RunInput {
    RunInput { ask: ask.into(), max_rounds: 2, base: "ea73cb05d639b945e21c667e17103e56404861a8".into(), max_in_flight: 4, campaign: None }
}

const NONCE: &str = "0123456789abcdef0123456789abcdef";
const UUID: &str = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";

#[test]
fn claude_has_every_capability() {
    let c = adapter_for(Engine::Claude).capabilities();
    assert!(c.pty_attach && c.transcript && c.inspect && c.message && c.redirect && c.resume && c.vajra_workflow);
    assert_eq!(c.stop, StopKind::HookBlock);
}

#[test]
fn codex_flags_are_honest() {
    let c = adapter_for(Engine::Codex).capabilities();
    assert!(c.pty_attach && c.resume);
    // No hook channel and no transcript parser for Codex: no steering, no inspector,
    // and a stop is a terminate, not a sticky block.
    assert!(!c.transcript && !c.inspect && !c.message && !c.redirect && !c.vajra_workflow);
    assert_eq!(c.stop, StopKind::Terminate);
}

#[test]
fn claude_run_argv_is_the_vajra_launch_and_never_bare() {
    let a = adapter_for(Engine::Claude).run_argv(&input("fix the login bug"), NONCE).unwrap();
    assert_eq!(a[0], "claude");
    assert!(a.iter().any(|s| s.starts_with("/vajra-deliver ")));
    assert!(!a.iter().any(|s| s == "--bare"));
}

#[test]
fn codex_run_argv_keeps_the_ask_one_positional_after_double_dash() {
    for ask in ["exec rm -rf /", "--dangerously-bypass-approvals-and-sandbox", "login"] {
        let a = adapter_for(Engine::Codex).run_argv(&input(ask), NONCE).unwrap();
        assert_eq!(a, vec!["codex".to_string(), "--".into(), ask.into()], "{ask}");
    }
    assert!(adapter_for(Engine::Codex).run_argv(&input("  "), NONCE).is_err());
    assert!(adapter_for(Engine::Codex).run_argv(&input(&"x".repeat(40 * 1024)), NONCE).is_err());
}

#[test]
fn codex_run_argv_never_carries_the_nonce() {
    let a = adapter_for(Engine::Codex).run_argv(&input("do it"), NONCE).unwrap();
    assert!(!a.iter().any(|s| s.contains(NONCE)));
}

#[test]
fn resume_argv_takes_only_a_uuid() {
    // The Claude resume is the live one: the campaign continues through
    // /vajra-deliver with the rotated nonce (not a bare "/vajra continue").
    let c = adapter_for(Engine::Claude).resume_argv(UUID, "20261105-1230", "deliver-20261105-1230", NONCE).unwrap();
    assert_eq!(&c[..3], ["claude", "--resume", UUID]);
    assert!(c[3].starts_with("/vajra-deliver ") && c[3].contains(NONCE), "{c:?}");
    assert_eq!(adapter_for(Engine::Codex).resume_argv(UUID, "20261105-1230", "c", NONCE).unwrap(), ["codex", "resume", UUID]);
    for bad in ["", "--last", "../x", "1b4e28ba-2fa1-41d2-883f-0016d3cca42Z", "1b4e28ba2fa141d2883f0016d3cca427"] {
        assert!(adapter_for(Engine::Claude).resume_argv(bad, "20261105-1230", "c", NONCE).is_err(), "{bad}");
        assert!(adapter_for(Engine::Codex).resume_argv(bad, "20261105-1230", "c", NONCE).is_err(), "{bad}");
    }
}

fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
    move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| OsString::from(*v))
}

#[test]
fn billing_is_the_login_because_keys_are_never_forwarded() {
    let b = adapter_for(Engine::Claude).billing(&env(&[("PATH", "/bin")]));
    assert_eq!(b, Billing { source: "login", key_in_app_env: None });
    // A key in the app's environment is reported by name only: the scrubbed child
    // environment does not carry it, so the CLI bills through its own login/config.
    let b = adapter_for(Engine::Claude).billing(&env(&[("ANTHROPIC_API_KEY", "sk-ant-SECRET")]));
    assert_eq!(b, Billing { source: "login", key_in_app_env: Some("ANTHROPIC_API_KEY") });
    let b = adapter_for(Engine::Codex).billing(&env(&[("CODEX_API_KEY", "sk-SECRET")]));
    assert_eq!(b.key_in_app_env, Some("CODEX_API_KEY"));
    let b = adapter_for(Engine::Codex).billing(&env(&[("OPENAI_API_KEY", "sk-SECRET")]));
    assert_eq!(b.key_in_app_env, Some("OPENAI_API_KEY"));
    // Empty means unset.
    let b = adapter_for(Engine::Claude).billing(&env(&[("ANTHROPIC_API_KEY", "")]));
    assert_eq!(b.key_in_app_env, None);
    // The value never appears anywhere in the result.
    let b = adapter_for(Engine::Claude).billing(&env(&[("ANTHROPIC_AUTH_TOKEN", "tok-SECRET")]));
    assert!(!format!("{b:?}").contains("SECRET"));
    assert_eq!(b.key_in_app_env, Some("ANTHROPIC_AUTH_TOKEN"));
}

#[test]
fn engine_ids_are_stable_for_the_inspector() {
    assert_eq!(Engine::Claude.id(), "claude");
    assert_eq!(Engine::Codex.id(), "codex");
    assert_eq!(Engine::from_id("codex"), Some(Engine::Codex));
    assert_eq!(Engine::from_id("gpt"), None);
}
