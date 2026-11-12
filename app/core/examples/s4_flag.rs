//! Spike S4 helper: print the Claude worker flag file `adapter::profiles::claude_worker_flag` builds, so the spike's
//! data-dir run uses the exact deny rules, sandbox block and guard hooks a real worker gets, not a hand-written copy.
//!
//!   cargo run -q --example s4_flag -- <home> <worktree> <data dir> <ticket tmp>
//!   cargo run -q --example s4_flag -- --shape     (the flag shape sha256 for `CLAUDE_WORKER_PROVEN`)
use std::path::PathBuf;

fn main() {
    let a: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if a.len() == 1 && a[0].as_os_str() == "--shape" {
        match vajra_desktop::adapter::profiles::claude_worker_flag_shape() {
            Ok(s) => return println!("{s}"),
            Err(e) => {
                eprintln!("s4_flag: {e}");
                std::process::exit(2);
            }
        }
    }
    let [home, work, data, tmp] = a.as_slice() else {
        eprintln!("usage: s4_flag <home> <worktree> <data dir> <ticket tmp>");
        std::process::exit(2);
    };
    match vajra_desktop::adapter::profiles::claude_worker_flag(home, work, &[], data, tmp) {
        Ok(s) => print!("{s}"),
        Err(e) => {
            eprintln!("s4_flag: {e}");
            std::process::exit(2);
        }
    }
}
