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
use vajra_desktop::terminal::{Event as TermEvent, Sink as TermSink, Terminals, UserPathEnv};

/// The one event the UI listens to.
const CHANGED: &str = "changed";

fn is_mcp(args: &[String]) -> bool {
    args.iter().skip(1).any(|a| a == "--mcp")
}

// -------------------------------------------------------------------- commands

#[tauri::command(async)]
fn inbox(app: State<'_, App>) -> Res {
    app.inbox()
}

#[tauri::command(async)]
fn approve_plan(app: State<'_, App>, id: String, plan_sha256: String) -> Res {
    app.approve_plan(&id, &plan_sha256)
}

#[tauri::command(async)]
fn reject_plan(app: State<'_, App>, id: String, reason: String) -> Res {
    app.reject_plan(&id, &reason)
}

#[tauri::command(async)]
fn preflight(app: State<'_, App>) -> Res {
    app.preflight()
}

/// A verify blocks on a subprocess and on the four verify slots, and the History screen asks for every campaign at
/// once. `#[tauri::command(async)]` runs the body on a runtime worker, so it would occupy workers while it waits; the
/// blocking pool takes it instead and the runtime stays free for the other commands and for event delivery.
#[tauri::command]
async fn campaign_integrity(handle: tauri::AppHandle, slug: String, refresh: Option<bool>) -> Res {
    tauri::async_runtime::spawn_blocking(move || handle.state::<App>().campaign_integrity(&slug, refresh.unwrap_or(false)))
        .await
        .unwrap_or_else(|e| Err(serde_json::json!({"error": "internal", "detail": e.to_string()})))
}

#[tauri::command(async)]
fn list_campaigns(app: State<'_, App>, refresh: Option<bool>) -> Res {
    app.list_campaigns_opt(refresh.unwrap_or(false))
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
fn read_report(app: State<'_, App>, slug: String) -> Res {
    app.read_report(&slug)
}

#[tauri::command(async)]
fn verify_campaign(app: State<'_, App>, slug: String) -> Res {
    app.verify_campaign(&slug)
}

#[tauri::command(async)]
fn resume_targets(app: State<'_, App>) -> Res {
    app.resume_targets()
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
fn stream_window(app: State<'_, App>, stream: String, from: Option<u64>, max_lines: usize, max_bytes: usize) -> Res {
    app.stream_window(&stream, from, max_lines, max_bytes)
}

#[tauri::command(async)]
fn read_spill_before(app: State<'_, App>, stream: String, before_seq: u64, max_lines: usize) -> Res {
    app.read_spill_before(&stream, before_seq, max_lines)
}

#[tauri::command(async)]
fn task(app: State<'_, App>, slug: String, task_id: String) -> Res {
    app.task(&slug, &task_id)
}

#[tauri::command(async)]
fn worktree_info(app: State<'_, App>, worker: String) -> Res {
    app.worktree_info(&worker)
}

#[tauri::command(async)]
fn evidence_list(app: State<'_, App>, slug: String) -> Res {
    app.evidence_list(&slug)
}

#[tauri::command(async)]
fn run_overview(app: State<'_, App>, slug: String) -> Res {
    app.run_overview(&slug)
}

#[tauri::command(async)]
fn evidence_detail(app: State<'_, App>, slug: String, task_id: String) -> Res {
    app.evidence_detail(&slug, &task_id)
}

#[tauri::command(async)]
fn diff(app: State<'_, App>, cwd: String, base: String) -> Res {
    app.diff(&cwd, &base)
}

// User terminals (module `terminal`): off the bus, the control service and MCP, so no agent can reach them. Output goes
// to the tab's own channel as raw bytes (an ArrayBuffer in the webview); the end of the shell is one JSON message
// {"exit": code}. The core checks the cwd against the roots it knows and starts the login shell as argv; the UI never
// names a program.

fn term_sink(ch: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>) -> TermSink {
    use tauri::ipc::InvokeResponseBody as Body;
    std::sync::Arc::new(move |e| {
        let _ = match e {
            TermEvent::Data(d) => ch.send(Body::Raw(d)),
            TermEvent::Exit(c) => ch.send(Body::Json(format!("{{\"exit\":{}}}", c.map_or("null".into(), |c| c.to_string())))),
        };
    })
}

fn term_err(e: vajra_desktop::terminal::TermError) -> serde_json::Value {
    serde_json::json!({"error": e.code(), "detail": e.to_string()})
}

fn term_value(i: &vajra_desktop::terminal::TermInfo) -> serde_json::Value {
    serde_json::to_value(i).unwrap_or_default()
}

/// Async (off the main thread) because it forks a shell.
#[tauri::command(async)]
fn term_open(
    app: State<'_, App>,
    terms: State<'_, Terminals>,
    cwd: String,
    rows: u16,
    cols: u16,
    on_event: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
) -> Res {
    terms.open(std::path::Path::new(&cwd), &app.terminal_roots(), rows, cols, term_sink(on_event)).map(|i| term_value(&i)).map_err(term_err)
}

/// Re-bind a terminal that outlived its view (the webview reloaded, or a hidden dock came back).
#[tauri::command]
fn term_attach(terms: State<'_, Terminals>, id: u32, on_event: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>) -> Res {
    terms.attach(id, term_sink(on_event)).map(|i| term_value(&i)).map_err(term_err)
}

/// Deliberately a plain (main-thread) command: sync commands run in arrival order, so typed bytes cannot be reordered, and
/// `write` only takes a short lock and queues, so it never blocks. (An async command is a task per call; two in flight
/// could swap.)
#[tauri::command]
fn term_write(terms: State<'_, Terminals>, id: u32, data: String) -> Res {
    terms.write(id, data.as_bytes()).map(|()| serde_json::json!({})).map_err(term_err)
}

#[tauri::command]
fn term_resize(terms: State<'_, Terminals>, id: u32, rows: u16, cols: u16) -> Res {
    terms.resize(id, rows, cols).map(|()| serde_json::json!({})).map_err(term_err)
}

#[tauri::command]
fn term_ack(terms: State<'_, Terminals>, id: u32, bytes: usize) -> Res {
    terms.ack(id, bytes).map(|()| serde_json::json!({})).map_err(term_err)
}

#[tauri::command(async)]
fn term_close(terms: State<'_, Terminals>, id: u32) -> Res {
    terms.close(id).map(|()| serde_json::json!({})).map_err(term_err)
}

#[tauri::command]
fn term_list(terms: State<'_, Terminals>) -> Res {
    Ok(serde_json::json!({"terminals": terms.list().iter().map(term_value).collect::<Vec<_>>()}))
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
fn ask_lead(app: State<'_, App>, run_id: String, text: String) -> Res {
    app.ask_lead(&run_id, &text)
}

#[tauri::command(async)]
fn cancel_ask(app: State<'_, App>, ask_id: String) -> Res {
    app.cancel_ask(&ask_id)
}

#[tauri::command(async)]
fn lead_status(app: State<'_, App>, run_id: String) -> Res {
    app.lead_status(&run_id)
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
    // A Finder/Dock launch has the minimal GUI PATH: ask the login shell once (bounded by userpath::TIMEOUT).
    vajra_desktop::userpath::init(&home);
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
        .manage(Terminals::with(Default::default(), std::sync::Arc::new(UserPathEnv::app(home.clone()))))
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
            campaign_integrity,
            start_run,
            resume_run,
            verify_campaign,
            read_report,
            resume_targets,
            crew,
            tail,
            read_spill,
            read_spill_before,
            stream_window,
            task,
            diff,
            evidence_list,
            evidence_detail,
            run_overview,
            worktree_info,
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
            ask_lead,
            cancel_ask,
            lead_status,
            inbox,
            approve_plan,
            reject_plan,
            term_open,
            term_attach,
            term_list,
            term_write,
            term_resize,
            term_ack,
            term_close,
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
        RunEvent::Exit => {
            let app = handle.state::<App>();
            app.shutdown_lead();
            app.sup.shutdown();
            handle.state::<Terminals>().shutdown();
        }
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
