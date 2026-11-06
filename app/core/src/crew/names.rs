//! Engine id -> UI name. The table is data (`names.tsv`); ids not in it are
//! title-cased (AC-2.1.3), so no engine name reaches the UI by default.

use std::collections::HashMap;
use std::sync::OnceLock;

const TABLE: &str = include_str!("names.tsv");

fn table() -> &'static HashMap<&'static str, &'static str> {
    static T: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    T.get_or_init(|| {
        TABLE
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .filter_map(|l| l.split_once('\t'))
            .collect()
    })
}

pub fn ui_name(engine_id: &str) -> String {
    match table().get(engine_id) {
        Some(name) => (*name).to_string(),
        None => title_case(engine_id),
    }
}

/// `foo-bar` -> `Foo Bar`; `-`, `_`, `.` and whitespace separate words.
pub fn title_case(id: &str) -> String {
    id.split(|c: char| c == '-' || c == '_' || c == '.' || c.is_whitespace())
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut cs = w.chars();
            match cs.next() {
                Some(f) => f.to_uppercase().chain(cs).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
