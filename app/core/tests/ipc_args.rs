//! The argument names of the stored-key commands, as the UI sends them. Tauri maps a Rust parameter `account_id` to the
//! JS key `accountId`; a name that differs on either side makes the call fail at run time, with every unit test green
//! (the UI once sent `accountId` to a command that took `account`). Both sides check the same file,
//! `tests/fixtures/ipc/key-commands.json`: this test reads the command signatures in `src/main.rs`, and
//! `app/ui/tests/keyCommandArgs.test.ts` reads what `api.providerKey*` actually sends.

use std::collections::BTreeMap;

fn camel(snake: &str) -> String {
    let mut out = String::new();
    let mut up = false;
    for c in snake.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// `fn <name>(...)` of every `provider_key_*` command in main.rs: its argument names in JS spelling, State/AppHandle excluded.
fn commands_in_main() -> BTreeMap<String, Vec<String>> {
    let src = include_str!("../src/main.rs");
    let mut found = BTreeMap::new();
    for (at, _) in src.match_indices("fn provider_key_") {
        let rest = &src[at + 3..];
        let name = &rest[..rest.find('(').unwrap()];
        let params = &rest[rest.find('(').unwrap() + 1..];
        let params = &params[..params.find(')').unwrap()];
        let params = params.replace("State<'_, App>", "State");
        let args: Vec<String> = params
            .split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .filter_map(|p| {
                let (n, ty) = p.split_once(':')?;
                (ty.trim() != "State").then(|| camel(n.trim()))
            })
            .collect();
        found.insert(name.to_string(), args);
    }
    found
}

#[test]
fn the_key_commands_take_exactly_the_arguments_the_ui_sends() {
    let want: BTreeMap<String, Vec<String>> = serde_json::from_str(include_str!("fixtures/ipc/key-commands.json")).unwrap();
    assert_eq!(commands_in_main(), want, "main.rs and tests/fixtures/ipc/key-commands.json must agree (and the UI test reads the same file)");
}
