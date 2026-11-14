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
use std::time::Instant;

use tauri::{Emitter, Manager, RunEvent, State, WindowEvent};

use vajra_desktop::control::{self, mcp};
use serde_json::Value;
use vajra_desktop::desktop::{App, Envelope, Pace, Res, collect_changes};
use vajra_desktop::logging;
use vajra_desktop::paths;
use vajra_desktop::sessions::argv::RunInput;
use vajra_desktop::owned::{Found, Ledger, Owner};
use vajra_desktop::desktop::WorktreeTerminals;
use vajra_desktop::terminal::{Event as TermEvent, Sink as TermSink, Terminals, UserPathEnv};

/// The one event the UI listens to.
const CHANGED: &str = "changed";

fn is_mcp(args: &[String]) -> bool {
    args.iter().skip(1).any(|a| a == "--mcp")
}

/// `--mcp-run`: a run lead's `vajra-run` server (ADR-16 16.5).
fn is_mcp_run(args: &[String]) -> bool {
    args.iter().skip(1).any(|a| a == "--mcp-run")
}

// -------------------------------------------------------------------- commands

#[tauri::command(async)]
fn preferences_read(app: State<'_, App>, project_id: Option<String>) -> Res {
    app.preferences_read(project_id.as_deref())
}
#[tauri::command(async)]
fn act_preferences_update(app: State<'_, App>, envelope: Envelope, patch: vajra_desktop::preferences::Patch, reset: bool) -> Value {
    app.act_preferences_update(&envelope, patch, reset)
}

#[tauri::command(async)]
fn inbox(app: State<'_, App>) -> Res {
    app.inbox()
}

#[tauri::command(async)]
fn approve_plan(app: State<'_, App>, id: String, plan_sha256: String) -> Res {
    app.approve_plan(&id, &plan_sha256)
}

/// Receipt-wrapped actions (FG-11): the answer is always a `vajra.receipt/1`; see `desktop::state`.
#[tauri::command(async)]
fn act_approve_plan(app: State<'_, App>, envelope: Envelope, id: String, plan_sha256: String) -> Value {
    app.act_approve_plan(&envelope, &id, &plan_sha256)
}

#[tauri::command(async)]
fn act_reject_plan(app: State<'_, App>, envelope: Envelope, id: String, reason: String, plan_sha256: String) -> Value {
    app.act_reject_plan(&envelope, &id, &reason, &plan_sha256)
}

#[tauri::command(async)]
fn act_answer_question(app: State<'_, App>, envelope: Envelope, id: String, text: String) -> Value {
    app.act_answer_question(&envelope, &id, &text)
}

#[tauri::command(async)]
fn act_stop_worker(app: State<'_, App>, envelope: Envelope, worker: String, reason: Option<String>) -> Value {
    app.act_stop_worker(&envelope, &worker, reason.as_deref())
}

/// The user terminals as closing a worker sees them: how many in a worktree are running something, and hanging them up
/// (ledger-recorded like any tab close). A close that fails keeps the worker open.
struct WorktreeTerms<'a>(&'a Terminals, &'a Ownership);

impl WorktreeTerminals for WorktreeTerms<'_> {
    fn busy(&self, wt: &std::path::Path) -> u32 {
        self.0.ids_under(wt).into_iter().filter(|&id| self.0.is_busy(id)).count() as u32
    }
    fn release(&self, wt: &std::path::Path) -> Result<(), String> {
        for id in self.0.ids_under(wt) {
            match self.1.ledger.close_then_forget(id, || self.0.close(id)) {
                Ok(()) | Err(vajra_desktop::terminal::TermError::NotFound) => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(())
    }
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
fn act_close_worker(app: State<'_, App>, terms: State<'_, Terminals>, own: State<'_, Ownership>, envelope: Envelope, worker: String, changed_files: Option<u32>, unpushed_commits: Option<u32>, busy_terminals: Option<u32>, no_upstream: Option<bool>) -> Value {
    // "Close anyway" sends everything the human confirmed; all four or none.
    let anyway = changed_files.zip(unpushed_commits).zip(busy_terminals).zip(no_upstream).map(|(((f, c), b), n)| (f, c, b, n));
    app.act_close_worker(&envelope, &worker, anyway, &WorktreeTerms(&terms, &own))
}

/// Preflight's "Install developer tools" (git is missing): opens Apple's installer.
#[tauri::command(async)]
fn install_dev_tools(app: State<'_, App>) -> Res {
    app.install_dev_tools()
}

/// Every ended core run, with whether it can be resumed and why not (6d2d). Read-only; off the event thread, since it
/// reads each run's records.
#[tauri::command(async)]
fn core_runs(app: State<'_, App>) -> Res {
    app.core_runs()
}

/// Labs for one project (ADR-16 16.13): whether this build has Labs, and whether it is on. Developer builds only.
#[tauri::command(async)]
fn labs_get(app: State<'_, App>, project: String) -> Res {
    app.labs_get(&project)
}

/// Turn Labs on or off for one project. A release build refuses.
#[tauri::command(async)]
fn labs_set(app: State<'_, App>, project: String, on: bool) -> Res {
    app.labs_set(&project, on)
}

/// Settings: the disk the worker copies use. A walk of the copies' folder, off the event thread.
#[tauri::command(async)]
fn worker_disk(app: State<'_, App>) -> Res {
    app.worker_disk()
}

#[tauri::command(async)]
fn act_worker_clean_up(app: State<'_, App>, envelope: Envelope) -> Value {
    app.act_worker_clean_up(&envelope)
}

/// Core runs whose signed record is still open after the app life that ran them (PR 6h).
#[tauri::command(async)]
fn record_orphans(app: State<'_, App>) -> Value {
    app.record_orphans(&[])
}

/// A core run's log entries after `after_seq` (W2): the UI applies these deltas in order instead of refetching.
#[tauri::command(async)]
fn run_log_since(app: State<'_, App>, run_id: String, after_seq: u64, limit: Option<usize>) -> Res {
    vajra_desktop::desktop::run_log_answered(app.run_log_since(&run_id, after_seq, limit.unwrap_or(vajra_desktop::desktop::runlog_store::PAGE_MAX)))
}

/// How far a core run's hash-chained log holds, and how far its signed record vouches for it (W2).
#[tauri::command(async)]
fn run_log_verify(app: State<'_, App>, run_id: String) -> Res {
    vajra_desktop::desktop::run_log_answered(app.run_log_verify(&run_id))
}

#[tauri::command(async)]
fn act_close_orphan(app: State<'_, App>, envelope: Envelope, run_id: String) -> Value {
    app.act_close_orphan(&envelope, &run_id)
}

#[tauri::command(async)]
fn act_close_finished(app: State<'_, App>, terms: State<'_, Terminals>, own: State<'_, Ownership>, envelope: Envelope) -> Value {
    app.act_close_finished(&envelope, &WorktreeTerms(&terms, &own))
}

#[tauri::command(async)]
fn closeout_status(app: State<'_, App>, terms: State<'_, Terminals>, own: State<'_, Ownership>, slug: String, run_id: String) -> Res {
    app.closeout_status(&slug, &run_id, &WorktreeTerms(&terms, &own))
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
fn act_start_run(app: State<'_, App>, envelope: Envelope, ask: String, max_rounds: u32, base: String, max_in_flight: u32, campaign: Option<String>, cwd: String, provider: Option<String>) -> Value {
    app.act_start_run(&envelope, RunInput { ask, max_rounds, base, max_in_flight, campaign, provider }, &cwd)
}

/// The Director-General's actions as receipts (FG-02c, decision Q2); see `desktop::lead_acts`.
#[tauri::command(async)]
fn act_ask_lead(app: State<'_, App>, envelope: Envelope, run_id: String, text: String) -> Value {
    app.act_ask_lead(&envelope, &run_id, &text)
}

#[tauri::command(async)]
fn act_new_conversation(app: State<'_, App>, envelope: Envelope, run_id: String) -> Value {
    app.act_new_conversation(&envelope, &run_id)
}

#[tauri::command(async)]
fn act_cancel_ask(app: State<'_, App>, envelope: Envelope, ask_id: String) -> Value {
    app.act_cancel_ask(&envelope, &ask_id)
}

#[tauri::command(async)]
fn act_start_proposal(app: State<'_, App>, envelope: Envelope, id: String, proposal_sha256: String, overrides: Option<Vec<Value>>, approve_checker: Option<bool>) -> Value {
    let opts = vajra_desktop::desktop::StartOptions { overrides: overrides.unwrap_or_default(), approve_checker: approve_checker.unwrap_or(false) };
    app.act_start_proposal_opts(&envelope, &id, &proposal_sha256, opts)
}

#[tauri::command(async)]
fn act_dismiss_proposal(app: State<'_, App>, envelope: Envelope, id: String) -> Value {
    app.act_dismiss_proposal(&envelope, &id)
}

#[tauri::command(async)]
fn project_state(app: State<'_, App>, project_id: String, since_rev: Option<u64>) -> Res {
    app.project_state(&project_id, since_rev)
}

#[tauri::command(async)]
fn reject_plan(app: State<'_, App>, id: String, reason: String) -> Res {
    app.reject_plan(&id, &reason)
}

#[tauri::command(async)]
fn app_info(app: State<'_, App>) -> Res {
    app.app_info()
}

#[tauri::command(async)]
fn project_for_folder(app: State<'_, App>, path: String) -> Res {
    app.project_for_folder(&path)
}

#[tauri::command(async)]
fn project_discover(app: State<'_, App>, extra: Option<Vec<String>>) -> Res {
    app.project_discover(&extra.unwrap_or_default())
}

#[tauri::command(async)]
fn project_list(app: State<'_, App>, include_archived: Option<bool>) -> Res {
    app.project_list(include_archived.unwrap_or(false))
}

#[tauri::command(async)]
fn project_add(app: State<'_, App>, path: String, confirm_nested: Option<bool>) -> Res {
    app.project_add(&path, confirm_nested.unwrap_or(false))
}

#[tauri::command(async)]
fn project_rename(app: State<'_, App>, id: String, name: String, expect_rev: Option<u64>) -> Res {
    app.project_rename(&id, &name, expect_rev)
}

#[tauri::command(async)]
fn project_archive(app: State<'_, App>, id: String, expect_rev: Option<u64>) -> Res {
    app.project_archive(&id, expect_rev)
}

#[tauri::command(async)]
fn project_sessions(app: State<'_, App>, id: String) -> Res {
    app.project_sessions(&id)
}

#[tauri::command(async)]
fn project_migration_suggest(app: State<'_, App>) -> Res {
    app.project_migration_suggest()
}

#[tauri::command(async)]
fn project_migration_confirm(app: State<'_, App>, roots: Vec<String>) -> Res {
    app.project_migration_confirm(&roots)
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
    provider: Option<String>,
) -> Res {
    app.start_run(RunInput { ask, max_rounds, base, max_in_flight, campaign, provider }, &cwd)
}

#[tauri::command(async)]
fn resume_run(app: State<'_, App>, session_id: String, campaign: String, cwd: String, provider: Option<String>) -> Res {
    app.resume_run(&session_id, &campaign, &cwd, provider.as_deref())
}

#[tauri::command(async)]
fn provider_list(app: State<'_, App>) -> Res {
    app.provider_list()
}

#[tauri::command(async)]
fn provider_accounts(app: State<'_, App>) -> Res {
    app.provider_accounts()
}

/// Stored API keys (D3). The key arrives once, goes to the Keychain through stdin, and is never sent back: every answer
/// is `{account, saved}`.
#[tauri::command(async)]
fn provider_key_saved(app: State<'_, App>, account_id: String) -> Res {
    app.provider_key_saved(&account_id)
}

#[tauri::command(async)]
fn provider_key_save(app: State<'_, App>, account_id: String, key: String) -> Res {
    app.provider_key_save(&account_id, key)
}

#[tauri::command(async)]
fn provider_key_replace(app: State<'_, App>, account_id: String, key: String) -> Res {
    app.provider_key_replace(&account_id, key)
}

#[tauri::command(async)]
fn provider_key_remove(app: State<'_, App>, account_id: String) -> Res {
    app.provider_key_remove(&account_id)
}

#[tauri::command(async)]
fn provider_usage(app: State<'_, App>, page_open: bool) -> Res {
    app.provider_usage(page_open)
}

#[tauri::command(async)]
fn read_report(app: State<'_, App>, slug: String) -> Res {
    app.read_report(&slug)
}

#[tauri::command(async)]
fn run_proof(app: State<'_, App>, slug: String) -> Res {
    app.run_proof(&slug)
}

#[tauri::command(async)]
fn export_proof_page(app: State<'_, App>, slug: String) -> Res {
    app.export_proof_page(&slug)
}

#[tauri::command(async)]
fn act_post_proof_comment(app: State<'_, App>, envelope: Envelope, slug: String, pr_url: String) -> Value {
    app.act_post_proof_comment(&envelope, &slug, &pr_url)
}

#[tauri::command(async)]
fn evidence_approval(app: State<'_, App>, run_id: String) -> Res {
    app.evidence_approval(&run_id)
}

#[tauri::command(async)]
fn act_answer_evidence_approval(app: State<'_, App>, envelope: Envelope, run_id: String, sha256: String, approve: bool) -> Value {
    app.act_answer_evidence_approval(&envelope, &run_id, &sha256, approve)
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
        let sent = match e {
            TermEvent::Data(d) => ch.send(Body::Raw(d)),
            TermEvent::Exit(c) => ch.send(Body::Json(format!("{{\"exit\":{}}}", c.map_or("null".into(), |c| c.to_string())))),
            TermEvent::Skipped { bytes, alt } => ch.send(Body::Json(format!("{{\"skipped\":{bytes},\"alt\":{}}}", alt.map_or("null".into(), |a| a.to_string())))),
            TermEvent::Activity => ch.send(Body::Json("{\"activity\":true}".into())),
        };
        sent.is_ok() // false once the webview is gone: the core then keeps the newest output for the view that attaches
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
    own: State<'_, Ownership>,
    cwd: String,
    rows: u16,
    cols: u16,
    on_event: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
) -> Res {
    let i = terms.open(std::path::Path::new(&cwd), &app.terminal_roots(), rows, cols, term_sink(on_event)).map_err(term_err)?;
    // Record who we started, with its start stamp, so a crash can be reconciled without trusting a bare pid.
    if let Some(stamp) = vajra_desktop::procid::start_stamp(i.pid)
        && own.record
    {
        let _ = own.ledger.record_for(i.id, Owner::shell(i.pid, stamp, i.cwd.clone()));
    }
    Ok(term_value(&i))
}

/// Settings > Providers > Set up: each catalog harness's state and the vendor's own install steps (ADR-17 step 1).
#[tauri::command(async)]
fn setup_list(app: State<'_, App>) -> Res {
    app.setup_list()
}

/// The one-click plan for a catalog harness, or why it is "Install it yourself" (ADR-17 rule 3). Main window only: the
/// app manifest guards it and only the main window's capability allows it.
#[tauri::command(async)]
fn setup_plan(app: State<'_, App>, id: String) -> Res {
    app.setup_plan(&id)
}

/// The user's Install click on the plan whose hash is `hash`: a single-use consent token (ADR-17 rule 4). Main window only.
#[tauri::command(async)]
fn setup_consent(app: State<'_, App>, id: String, hash: String) -> Res {
    app.setup_consent(&id, &hash)
}

/// Run the consented install; its output streams on `setup-<id>`. Main window only.
#[tauri::command(async)]
fn setup_run(app: State<'_, App>, id: String, token: String) -> Res {
    app.setup_run(&id, &token)
}

/// A CLI's own sign-in in a user terminal tab. The UI names a catalog harness id, never a program; the core builds the
/// argv from the compiled catalog and the binary detection found.
#[tauri::command(async)]
fn setup_sign_in(
    app: State<'_, App>,
    terms: State<'_, Terminals>,
    own: State<'_, Ownership>,
    id: String,
    rows: u16,
    cols: u16,
    on_event: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
) -> Res {
    let cmd = app.setup_sign_in_command(&id)?;
    let i = terms.open_sign_in(&cmd, rows, cols, term_sink(on_event)).map_err(term_err)?;
    if let Some(stamp) = vajra_desktop::procid::start_stamp(i.pid)
        && own.record
    {
        let _ = own.ledger.record_for(i.id, Owner::shell(i.pid, stamp, i.cwd.clone()));
    }
    Ok(term_value(&i))
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

/// The view shows or hides a pane (the dock hidden, another tab selected). `alt`: it was on the alternate screen when it hid.
#[tauri::command]
fn term_visible(terms: State<'_, Terminals>, id: u32, visible: bool, alt: Option<bool>) -> Res {
    terms.set_visible(id, visible, alt.unwrap_or(false)).map(|()| serde_json::json!({})).map_err(term_err)
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
fn term_close(terms: State<'_, Terminals>, own: State<'_, Ownership>, id: u32) -> Res {
    // The record goes after the shell is stopped (exactly the record this terminal made, not whatever shares its pid now):
    // until then a crash must still leave the shell recoverable.
    own.ledger.close_then_forget(id, || terms.close(id)).map(|()| serde_json::json!({})).map_err(term_err)
}

#[tauri::command]
fn term_list(terms: State<'_, Terminals>) -> Res {
    Ok(serde_json::json!({"terminals": terms.list().iter().map(term_value).collect::<Vec<_>>()}))
}

/// Shells recorded by an earlier session of the app, reconciled once at launch, and what the human did about them.
struct Ownership {
    ledger: std::sync::Arc<Ledger>,
    /// False in a second instance: it writes nothing to the ledger.
    record: bool,
    found: std::sync::Mutex<Vec<Found>>,
    home: String,
}

fn found_lock(own: &Ownership) -> std::sync::MutexGuard<'_, Vec<Found>> {
    own.found.lock().unwrap_or_else(|e| e.into_inner())
}

/// What keeps running after the window closes, after a quit and after an archive, and what the last crash left behind.
/// On demand: nothing polls it.
#[tauri::command]
fn lifecycle(app: State<'_, App>, terms: State<'_, Terminals>, own: State<'_, Ownership>) -> Res {
    let mut v = serde_json::to_value(vajra_desktop::lifecycle_report::report(&terms.list(), &app.active_run_diags())).map_err(internal)?;
    v["found"] = serde_json::to_value(&*found_lock(&own)).map_err(internal)?;
    // Disclosed (6c-3): what the startup sweep killed of an earlier app life's workers, and what each attempt's sweep
    // killed of descendants that left their group.
    v["swept"] = app.swept_report();
    Ok(v)
}

fn internal(e: serde_json::Error) -> serde_json::Value {
    serde_json::json!({"error": "internal", "detail": e.to_string()})
}

/// End leftover shells the human picked. Records that reconciled as owned, or as held but unproven, are eligible, and
/// each is proven again (pid and start stamp) right before it is signalled: an unproven one that still can't be proven
/// answers `not_proven` and is kept. Anything else is reported and left alone.
#[tauri::command(async)]
fn recover_orphans(own: State<'_, Ownership>, pids: Vec<u32>) -> Res {
    let picked: Vec<Found> = found_lock(&own).iter().filter(|f| pids.contains(&f.owner.pid) && f.verdict.keeps()).cloned().collect();
    let mut out = Vec::new();
    for f in &picked {
        let r = vajra_desktop::owned::recover(&f.owner);
        // Only an end (or a record that was not ours) settles it; members still running keep the record and the retry.
        vajra_desktop::owned::apply_recovery(&own.ledger, &mut found_lock(&own), &f.owner, &r);
        out.push(serde_json::json!({"pid": f.owner.pid, "result": r}));
    }
    for pid in pids.iter().filter(|p| !picked.iter().any(|f| f.owner.pid == **p)) {
        out.push(serde_json::json!({"pid": pid, "result": "not_owned"}));
    }
    Ok(serde_json::json!({"results": out}))
}

/// The redacted, bounded text the human may read and copy. Nothing is sent anywhere.
#[tauri::command]
fn diagnostics_preview(app: State<'_, App>, terms: State<'_, Terminals>, own: State<'_, Ownership>) -> Res {
    let report = vajra_desktop::lifecycle_report::report(&terms.list(), &app.active_run_diags());
    Ok(serde_json::json!({"text": vajra_desktop::diagnostics::preview(&report, &found_lock(&own), &own.home)}))
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

/// By run (`runId`) or, with no run, by project (`projectId`, FG-02c): exactly one of them.
#[tauri::command(async)]
fn lead_status(app: State<'_, App>, run_id: Option<String>, project_id: Option<String>) -> Res {
    match (run_id, project_id) {
        (Some(r), None) => app.lead_status(&r),
        (None, Some(p)) => app.lead_status_for_project(&p),
        _ => Err(serde_json::json!({"error": "invalid_args", "detail": "give runId or projectId"})),
    }
}

/// ADR-15 2.4: start a run a Director-General proposed, as the inbox showed it (`proposal_sha256`).
#[tauri::command(async)]
fn start_proposal(app: State<'_, App>, id: String, proposal_sha256: String, overrides: Option<Vec<Value>>, approve_checker: Option<bool>) -> Res {
    let opts = vajra_desktop::desktop::StartOptions { overrides: overrides.unwrap_or_default(), approve_checker: approve_checker.unwrap_or(false) };
    app.start_proposal_opts(&id, &proposal_sha256, opts)
}

/// ADR-15 2.4: dismiss a proposed run; nothing runs.
#[tauri::command(async)]
fn dismiss_proposal(app: State<'_, App>, id: String) -> Res {
    app.dismiss_proposal(&id)
}

/// ADR-15 2.5: the Director-General of the run's project starts a new conversation (generation n+1).
#[tauri::command(async)]
fn new_conversation(app: State<'_, App>, run_id: String) -> Res {
    app.new_conversation(&run_id)
}

#[tauri::command(async)]
fn active_runs(app: State<'_, App>) -> Res {
    app.active_runs()
}

/// ADR-15 D5: the run caps (1 per project, `max_runs` in the app, the worker ceiling).
#[tauri::command(async)]
fn run_limits(app: State<'_, App>) -> Res {
    app.run_limits()
}

#[tauri::command(async)]
fn set_run_limits(app: State<'_, App>, max_runs: u32) -> Res {
    app.set_run_limits(max_runs)
}

// -------------------------------------------------------------------- main

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if is_mcp_run(&args) {
        // Before Tauri and before the data dir: the run server is told its socket and creates nothing on disk.
        let code = match mcp::run_stdio_run() {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("vajra-desktop --mcp-run: {e}");
                1
            }
        };
        std::process::exit(code);
    }
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
    // A Finder/Dock launch has the minimal GUI PATH: ask the login shell once (bounded by userpath::TIMEOUT), in the
    // background so the window does not wait for it. The first reader of the PATH waits for the answer instead.
    vajra_desktop::userpath::start(&home);
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
    // Only the app holding the instance locks ticks the control ledger and serves the Lead socket (and the run leads'
    // vajra-run calls on it).
    let _watcher = app.serve_control();
    let sub = app.bus.subscribe();

    // Crash reconciliation: shells an earlier session recorded are checked against the live process table now (pid and
    // start stamp, never a name). Nothing is signalled until the human asks.
    // The App's ledger: one writer for shells, leads and workers alike.
    let owned_ledger = app.owned_ledger();
    let other_instance = app.other_instance();
    let built = tauri::Builder::default()
        // The system folder picker for the repository field (D5). The capability grants only `dialog:allow-open`.
        .plugin(tauri_plugin_dialog::init())
        .manage(app)
        // A second instance offers no shell for recovery (the first app's live terminals would look like a crash's) and
        // records none, so it never writes the other app's ledger.
        .manage(Ownership {
            found: std::sync::Mutex::new(if other_instance { Vec::new() } else { owned_ledger.settle() }),
            ledger: owned_ledger,
            home: home.display().to_string(),
            record: !other_instance,
        })
        .manage(Terminals::with(Default::default(), std::sync::Arc::new(UserPathEnv::app(home.clone()))))
        .setup(move |a| {
            let handle = a.handle().clone();
            // Receipts notify on their own: an action with no bus output still moves the projection.
            let sink_handle = handle.clone();
            handle.state::<App>().set_change_sink(Box::new(move |p| {
                let _ = sink_handle.emit(CHANGED, p);
            }));
            let event_handle = handle.clone();
            handle.state::<App>().set_event_sink(Box::new(move |name, p| {
                let _ = event_handle.emit(name, p);
            }));
            // Which accounts have a stored key is looked up once here, off every request path, so the usage row shows up
            // shortly after launch without opening the Providers page (a slow Keychain delays nothing else).
            let discover = handle.clone();
            std::thread::spawn(move || discover.state::<App>().discover_usage_accounts());
            std::thread::spawn(move || {
                #[cfg(target_os = "macos")]
                let mut nap = vajra_desktop::nap::NapGuard::new(vajra_desktop::nap::macos::ProcessInfo);
                // Nothing runs at rest: with no live run (and past `AFTER_RUN`), the loop waits on the bus. A line, the
                // wake a new run sends, or the `IDLE_WAIT` housekeeping tick brings it back. Launch starts busy.
                let mut pace = Pace::new(Instant::now());
                let mut wait = vajra_desktop::desktop::LIVE_WAIT;
                loop {
                    let changed = collect_changes(&sub, wait);
                    let (busy, next) = pace.next(handle.state::<App>().active_run_count() > 0, Instant::now());
                    wait = next;
                    // Workers are published before anyone is told: a subscriber that refetches on the event sees them.
                    if busy {
                        handle.state::<App>().refresh_workers();
                    }
                    // Finished workers that lose nothing are closed on their own (setting `auto_close_finished`). Git is
                    // asked off this loop, so a slow repo never delays worker refreshes. Not gated on `busy`: at rest a
                    // turn is an event or the `IDLE_WAIT` tick, so a worker that became safe later is still closed.
                    if handle.state::<App>().closeout_due() {
                        let h = handle.clone();
                        std::thread::spawn(move || {
                            let n = h.state::<App>().closeout_auto(&WorktreeTerms(&h.state::<Terminals>(), &h.state::<Ownership>()));
                            if n > 0 {
                                let _ = h.emit(CHANGED, serde_json::json!({"projects": h.state::<App>().bump_all_projects()}));
                            }
                        });
                    }
                    if let Some(mut payload) = changed {
                        // The revision each known project is now at, so a subscriber refetches `project_state`.
                        payload["projects"] = Value::Array(handle.state::<App>().bump_all_projects());
                        let _ = handle.emit(CHANGED, payload);
                    }
                    // ADR-9 item 7: no App Nap while a run is active.
                    #[cfg(target_os = "macos")]
                    {
                        let app = handle.state::<App>();
                        if busy {
                            app.reap_runs();
                        }
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
            preferences_read,
            act_preferences_update,
            preflight,
            list_campaigns,
            campaign_integrity,
            start_run,
            resume_run,
            provider_list,
            provider_accounts,
            setup_list,
            setup_sign_in,
            setup_plan,
            setup_consent,
            setup_run,
            provider_key_saved,
            provider_key_save,
            provider_key_replace,
            provider_key_remove,
            provider_usage,
            verify_campaign,
            read_report,
            run_proof,
            export_proof_page,
            act_post_proof_comment,
            evidence_approval,
            act_answer_evidence_approval,
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
            run_limits,
            set_run_limits,
            ask_lead,
            cancel_ask,
            lead_status,
            new_conversation,
            start_proposal,
            dismiss_proposal,
            inbox,
            approve_plan,
            reject_plan,
            app_info,
            project_list,
            project_for_folder,
            project_discover,
            project_add,
            project_rename,
            project_archive,
            project_sessions,
            project_migration_suggest,
            project_migration_confirm,
            act_approve_plan,
            act_reject_plan,
            act_answer_question,
            act_stop_worker,
            act_close_worker,
            act_close_finished,
            worker_disk,
            install_dev_tools,
            core_runs,
            labs_get,
            labs_set,
            act_worker_clean_up,
            record_orphans,
            run_log_since,
            run_log_verify,
            act_close_orphan,
            closeout_status,
            act_start_run,
            act_ask_lead,
            act_new_conversation,
            act_cancel_ask,
            act_start_proposal,
            act_dismiss_proposal,
            project_state,
            term_open,
            term_attach,
            term_list,
            lifecycle,
            recover_orphans,
            diagnostics_preview,
            term_write,
            term_resize,
            term_visible,
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
            // The Director-General's asks, then every core run's lead, then core tickets (with their descendants), then
            // every session: `App::quit`.
            handle.state::<App>().quit();
            handle.state::<Terminals>().shutdown();
            // A clean quit reaped every shell, so there is nothing to recover. A second instance leaves the ledger alone.
            handle.state::<App>().clear_ledger_on_quit();
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
        assert!(is_mcp_run(&a(&["vajra-desktop", "--mcp-run"])));
        assert!(!is_mcp_run(&a(&["vajra-desktop", "--mcp"])), "--mcp is the Lead chat server, never the run server");
        assert!(!is_mcp(&a(&["vajra-desktop", "--mcp-run"])));
    }
}
