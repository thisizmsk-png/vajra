//! Spike S4 helper: print the Claude worker flag file `adapter::profiles::claude_worker_flag` builds, so the spike's
//! data-dir run uses the exact deny rules and guard hooks a real worker gets, not a hand-written copy.
//!
//!   cargo run -q --example s4_flag -- <home> <worktree> <data dir>
use std::path::PathBuf;

fn main() {
    let a: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [home, work, data] = a.as_slice() else {
        eprintln!("usage: s4_flag <home> <worktree> <data dir>");
        std::process::exit(2);
    };
    match vajra_desktop::adapter::profiles::claude_worker_flag(home, work, &[], data) {
        Ok(s) => print!("{s}"),
        Err(e) => {
            eprintln!("s4_flag: {e}");
            std::process::exit(2);
        }
    }
}
