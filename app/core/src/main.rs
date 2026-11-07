//! Vajra Desktop entry point: a thin Tauri v2 shell over the C1-C6 core.
//!
//! `--mcp` runs the stdio MCP server (the Lead chat session's side of the
//! control socket) and never starts Tauri. Otherwise the app opens its data
//! dir, the bus, the PTY supervisor and the control service, serves the
//! owner-only socket, and registers one Tauri command per UI call. The UI
//! learns about new output through a single `changed` event and pulls the
//! data with `tail` / `read_spill`.
//!
//! ADR-9: closing the window hides it; runs keep going in this process.
//! The menu bar item and the Cmd-Q prompt are M4.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::time::Duration;

use tauri::{Emitter, Manager, RunEvent, State, WindowEvent};

use vajra_desktop::control::{self, mcp, service, socket};
use vajra_desktop::desktop::{App, Res, collect_changes};
use vajra_desktop::logging;
use vajra_desktop::paths;
use vajra_desktop::sessions::argv::RunInput;

/// The one event the UI listens to.
const CHANGED: &str = "changed";

fn is_mcp(args: &[String]) -> bool {
    args.iter().skip(1).any(|a| a == "--mcp")
}

// -------------------------------------------------------------------- commands

#[tauri::command(async)]
fn preflight(app: State<'_, App>) -> Res {
    app.preflight()
}

#[tauri::command(async)]
fn list_campaigns(app: State<'_, App>) -> Res {
    app.list_campaigns()
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
fn start_run(
    app: State<'_, App>,
    ask: String,
    max_rounds: u32,
    base: String,
    max_in_flight: u32,
    campaign: Option<String>,
    cwd: String,
) -> Res {
    app.start_run(RunInput { ask, max_rounds, base, max_in_flight, campaign }, &cwd)
}

#[tauri::command(async)]
fn resume_run(app: State<'_, App>, session_id: String, campaign: String, cwd: String) -> Res {
    app.resume_run(&session_id, &campaign, &cwd)
}

#[tauri::command(async)]
fn verify_campaign(app: State<'_, App>, slug: String) -> Res {
    app.verify_campaign(&slug)
}

#[tauri::command(async)]
fn read_report(app: State<'_, App>, slug: String) -> Res {
    app.read_report(&slug)
}

#[tauri::command(async)]
fn crew(app: State<'_, App>, slug: String, run_id: String) -> Res {
    app.crew(&slug, &run_id)
}

#[tauri::command(async)]
fn tail(app: State<'_, App>, stream: String, after_seq: u64, max_bytes: usize) -> Res {
    app.tail(&stream, after_seq, max_bytes)
}

#[tauri::command(async)]
fn read_spill(app: State<'_, App>, stream: String, offset: u64, max_bytes: usize) -> Res {
    app.read_spill(&stream, offset, max_bytes)
}

#[tauri::command(async)]
fn task(app: State<'_, App>, slug: String, task_id: String) -> Res {
    app.task(&slug, &task_id)
}

#[tauri::command(async)]
fn diff(app: State<'_, App>, cwd: String, base: String) -> Res {
    app.diff(&cwd, &base)
}

#[tauri::command(async)]
fn control_message(app: State<'_, App>, worker: String, text: String) -> Res {
    app.control_message(&worker, &text)
}

#[tauri::command(async)]
fn control_redirect(app: State<'_, App>, worker: String, text: String) -> Res {
    app.control_redirect(&worker, &text)
}

#[tauri::command(async)]
fn control_stop(app: State<'_, App>, worker: String, reason: Option<String>) -> Res {
    app.control_stop(&worker, reason.as_deref())
}

#[tauri::command(async)]
fn confirm_stop(app: State<'_, App>, action_id: String) -> Res {
    app.confirm_stop(&action_id)
}

#[tauri::command(async)]
fn cancel_stop(app: State<'_, App>, action_id: String) -> Res {
    app.cancel_stop(&action_id)
}

#[tauri::command(async)]
fn resume_worker(app: State<'_, App>, worker: String) -> Res {
    app.resume_worker(&worker)
}

#[tauri::command(async)]
fn ledger(app: State<'_, App>) -> Res {
    app.ledger()
}

#[tauri::command(async)]
fn kb_search(app: State<'_, App>, query: String, k: u32, repo: String) -> Res {
    app.kb_search(&query, k, &repo)
}

#[tauri::command(async)]
fn take_over(app: State<'_, App>, stream: String, on: bool) -> Res {
    app.take_over(&stream, on)
}

#[tauri::command(async)]
fn active_runs(app: State<'_, App>) -> Res {
    app.active_runs()
}

// -------------------------------------------------------------------- main

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let data = match paths::data_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("vajra-desktop: data dir: {e}");
            std::process::exit(1);
        }
    };
    if is_mcp(&args) {
        // Before Tauri: stdout belongs to the MCP protocol.
        let code = match mcp::run_stdio(&control::socket_path(&data)) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("vajra-desktop --mcp: {e}");
                1
            }
        };
        std::process::exit(code);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let app = match App::open(&data, &home) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("vajra-desktop: startup: {e}");
            std::process::exit(1);
        }
    };
    if let Some(l) = &app.log {
        let _ = l.log_cold_start(logging::process_start_time());
    }
    let _watcher = service::spawn_watcher(app.control.clone(), Duration::from_secs(1));
    match socket::bind(&control::socket_path(&data)) {
        Ok(listener) => {
            let svc = app.control.clone();
            // SAFETY: getuid has no preconditions and cannot fail.
            let uid = unsafe { libc::getuid() };
            std::thread::spawn(move || socket::serve(listener, svc, uid));
        }
        Err(e) => {
            eprintln!("vajra-desktop: control socket: {e}");
            app.note("control_socket_failed", &[]);
        }
    }
    let sub = app.bus.subscribe();

    let built = tauri::Builder::default()
        .manage(app)
        .setup(move |a| {
            let handle = a.handle().clone();
            std::thread::spawn(move || {
                #[cfg(target_os = "macos")]
                let mut nap = vajra_desktop::nap::NapGuard::new(vajra_desktop::nap::macos::ProcessInfo);
                loop {
                    if let Some(payload) = collect_changes(&sub, Duration::from_secs(1)) {
                        let _ = handle.emit(CHANGED, payload);
                    }
                    handle.state::<App>().refresh_workers();
                    // ADR-9 item 7: no App Nap while a run is active.
                    #[cfg(target_os = "macos")]
                    {
                        let app = handle.state::<App>();
                        app.reap_runs();
                        nap.update(app.active_run_count());
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // ADR-9: closing the window keeps runs going.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            preflight,
            list_campaigns,
            start_run,
            resume_run,
            verify_campaign,
            read_report,
            crew,
            tail,
            read_spill,
            task,
            diff,
            control_message,
            control_redirect,
            control_stop,
            confirm_stop,
            cancel_stop,
            resume_worker,
            ledger,
            kb_search,
            take_over,
            active_runs,
        ])
        .build(tauri::generate_context!());
    let tauri_app = match built {
        Ok(a) => a,
        Err(e) => {
            eprintln!("vajra-desktop: tauri: {e}");
            std::process::exit(1);
        }
    };
    tauri_app.run(|handle, event| match event {
        // Dock icon click after the window was hidden.
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => {
            if let Some(w) = handle.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
        RunEvent::Exit => handle.state::<App>().sup.shutdown(),
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_flag_is_found_only_after_argv0() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(is_mcp(&a(&["vajra-desktop", "--mcp"])));
        assert!(is_mcp(&a(&["vajra-desktop", "-x", "--mcp"])));
        assert!(!is_mcp(&a(&["--mcp"])));
        assert!(!is_mcp(&a(&["vajra-desktop", "--mcpx"])));
    }
}
