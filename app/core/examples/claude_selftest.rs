//! Run the canary A self-test (`adapter::claude_selftest`) by hand, as the app does for a Claude Code build that is
//! not on the proven list. `<home>` is where the Vajra guard hooks are installed (`<home>/.claude/skills/vajra/hooks`);
//! they are copied into a throwaway HOME, and nothing under `<home>` is changed.
//!
//!   cargo run -q --example claude_selftest -- <absolute claude binary> <home>
use std::path::PathBuf;

fn main() {
    let a: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [claude, home] = a.as_slice() else {
        eprintln!("usage: claude_selftest <absolute claude binary> <home>");
        std::process::exit(2);
    };
    let start = std::time::Instant::now();
    let r = vajra_desktop::adapter::claude_selftest::run(claude, home, vajra_desktop::adapter::claude_selftest::TIMEOUT);
    let secs = start.elapsed().as_secs_f32();
    match r {
        Ok(()) => println!("self-test PASS ({secs:.1} s)"),
        Err(e) => {
            println!("self-test FAIL: {e} ({secs:.1} s)");
            std::process::exit(1);
        }
    }
}
