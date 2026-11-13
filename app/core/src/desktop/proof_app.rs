//! The App side of a run's proof (6i, D6): gather it from the core's own reads, save the page, post the PR comment.
//!
//! Every text that leaves here has been cleaned and redacted twice: the campaign's nonce set, then the key-shaped and
//! secret-field pass. Raw ledger lines, env values and the key never reach it: only the goal, the criteria, the row
//! statuses, the commits, the seat that checked, the report text and the record hash do.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use super::proof::{self, Input, Label, Proof, Row};
use super::{bus, fail, invalid, App, Envelope, Res};
use crate::lock;
use crate::crew::redact::{redact_provider_text, Redactor};

/// The run id a `deliver-<run>` or `deliver-<run>-og` campaign belongs to.
fn run_of(slug: &str) -> Option<&str> {
    slug.strip_prefix("deliver-").map(|r| r.strip_suffix("-og").unwrap_or(r)).filter(|r| crate::ids::valid_run_id(r))
}

/// `gh pr comment` gets this long.
const POST_TIMEOUT: Duration = Duration::from_secs(30);
/// The largest comment GitHub takes is 65536 characters; the proof stays well under it.
const COMMENT_MAX: usize = 60 * 1024;

impl App {
    /// The run's proof for the Run view: its label (the worse of the signed marker and `seats.json`), the record check
    /// and hash, and the PR comment text. Read-only; nothing here is a network call.
    pub fn run_proof(&self, slug: &str) -> Res {
        let p = self.gather_proof(slug)?;
        let md = proof::markdown(&p);
        Ok(proof::proof_json(&p, &md))
    }

    fn gather_proof(&self, slug: &str) -> Result<Proof, Value> {
        self.gather_proof_with(slug, &|| {})
    }

    /// `between` runs after the held read, before the check (tests swap files there).
    pub(super) fn gather_proof_with(&self, slug: &str, between: &dyn Fn()) -> Result<Proof, Value> {
        if !bus::valid_stream_id(slug) {
            return Err(invalid("slug", "not a campaign slug"));
        }
        // The record check reads the held bytes and asks the script only about their digest (it holds the key). The
        // label's goal, status, slug and progress come from those same verified bytes, never from a second read: a
        // file swapped after the check can't change the label. With no verified campaign they are empty, and the label
        // fails closed (integrity is not `ok` then anyway).
        self.with_reader(|r| r.campaign(slug).map(drop).map_err(|e| fail("not_found", e)))?;
        let mut report = self.read_report_with(slug, &|_| {}, between)?;
        let verified = report.as_object_mut().and_then(|m| m.remove(super::VERIFIED_KEY)).unwrap_or(Value::Null);
        let integrity = report["integrity"].as_str().unwrap_or("unknown").to_string();
        let field = |k: &str| verified["campaign"][k].as_str().unwrap_or("").to_string();
        let (goal, status, slug_field) = (field("goal"), field("status"), field("slug"));
        let progress = verified["progress"].as_str().unwrap_or("");
        let snap = self.with_reader(|r| Ok(r.tasks(slug)))?;
        let tasks: Option<&[Value]> = snap.value.as_ref().and_then(|v| v.get("tasks")).and_then(Value::as_array).map(Vec::as_slice);
        let rejected = super::runview::timeline(progress, tasks).outcome == super::runview::Outcome::Rejected;
        // The same bounded check the Run view runs (cached verdicts are reused). A row it couldn't check reads
        // `checking`, which is not a pass, so the label can only come out weaker, never stronger.
        let list = self.evidence_list(slug).unwrap_or_else(|_| json!({"rows": []}));
        let rows_json = list["rows"].as_array().cloned().unwrap_or_default();
        let (trust, checker, consumed, key_readable) = self.trust_of(slug);
        // N20 (bhishma M1, Opus on #226): the consume this app holds in memory names the list, the core's record agrees
        // (`evidence_approval_consumed`), and a signed progress line is exactly the approval line for that list (the
        // whole line text after its timestamp, no prefix). Neither the file nor the line counts alone.
        let approval = consumed.is_some_and(|sha| {
            let line = super::evidence_approval::progress_line(&sha);
            progress.lines().filter_map(|l| l.strip_prefix("- ")).any(|l| l.split_once(' ').is_some_and(|(_, text)| text.trim_end() == line))
        });
        let statuses: Vec<&str> = rows_json.iter().map(|r| r["status"].as_str().unwrap_or("not_run")).collect();
        let (label, reason) = proof::label(&Input {
            slug,
            goal: &goal,
            status: &status,
            integrity: &integrity,
            rows: statuses,
            trust: trust.as_deref(),
            approval_consumed: approval,
            approval_unheld: !approval && run_of(slug).is_some_and(|r| self.evidence_approval_unheld(r)),
            rejected,
            slug_field: &slug_field,
            ledger: self.ledger_of(slug, &verified),
        });
        let red = self.campaign_redactor(slug);
        // Text only: the nonce set, secret fields, then key-shaped words and home paths. Commits and the record hash are
        // checked hex and kept as they are (the key-shape pass would hide a 40-hex commit).
        let home = self.home.to_string_lossy().into_owned();
        let clean = &|s: &str| crate::diagnostics::redact(&redact_provider_text(&red.clean(s)), &home);
        let rows = rows_json
            .iter()
            .map(|r| Row {
                criterion: clean(r["criterion"].as_str().unwrap_or("")),
                task: clean(r["task"].as_str().unwrap_or("")),
                status: r["status"].as_str().unwrap_or("not_run").to_string(),
                // A worker writes `head`: a nonce is hex too, so a value the nonce set would change is dropped, not kept.
                commit: r["head"].as_str().filter(|h| is_hex(h, 7..=64) && red.clean(h) == *h).map(str::to_string),
                ran_by: checker.as_deref().or_else(|| r["owner"].as_str()).map(clean),
            })
            .collect();
        let report_text = report["text"].as_str().map(|t| t.lines().map(&clean).collect::<Vec<_>>().join("\n"));
        Ok(Proof {
            slug: slug.to_string(),
            goal: clean(goal.trim_start().strip_prefix(proof::MARKER).unwrap_or(&goal).trim()),
            label,
            reason,
            integrity,
            record_sha256: report["record_sha256"].as_str().filter(|h| is_hex(h, 64..=64)).map(str::to_string),
            rows,
            report: report_text,
            key_readable,
        })
    }

    /// The core's own trust record for the campaign's run: `seats.json`'s trust, who checked (harness and account of the
    /// evidence seat) and whether a single-use approval was consumed. A campaign no run of this app started has none; one
    /// whose record can't be read answers `"?"`, so the label fails closed.
    ///
    /// The last value is true when a worker in the run could read the signing key: any seat that isn't a Claude seat
    /// (Codex can, by the owner's choice; an unknown harness counts too), or a seats record that can't be read or names
    /// another campaign. A campaign no run of this app started keeps today's note (bhishma M1 on #237).
    fn trust_of(&self, slug: &str) -> (Option<String>, Option<String>, Option<String>, bool) {
        let Some(run) = run_of(slug) else { return (None, None, None, false) };
        let seats = match self.read_run_seats(run) {
            Ok(Some(s)) => s,
            Ok(None) => return (None, None, None, false),
            Err(_) => return (Some("?".into()), None, None, true),
        };
        // The record must name this campaign; one that names another is not this run's.
        if seats["campaign_slug"].as_str() != Some(slug) {
            return (Some("?".into()), None, None, true);
        }
        let key_readable = seats["seats"].as_array().is_none_or(|all| all.iter().any(|s| s["harness"].as_str() != Some("claude")));
        let trust = seats["evidence_trust"].as_str().unwrap_or("?").to_string();
        let seat = seats["evidence_seat"].as_str();
        let checker = seats["seats"].as_array().and_then(|all| all.iter().find(|s| s["seat"].as_str() == seat)).map(|s| {
            let harness = s["harness"].as_str().unwrap_or("unknown harness");
            format!("{harness} ({})", s["account"].as_str().unwrap_or(""))
        });
        // N20: the list the core's own approval record says this run consumed (never a `seats.json` field); the caller
        // also needs the signed progress line.
        (Some(trust), checker, self.evidence_approval_consumed(run), key_readable)
    }

    /// The record against the core's own write ledger (6h, #190; bhishma's rule): the campaign, features and progress
    /// are the bytes the record check just verified (the same ones the label reads, so a signed swap between two
    /// reads can't pass one check and feed the other), with the task graph read by `list --json`; the ledger is the
    /// core's own file for this slug. A run with no ledger (one this app's core didn't record) is `Missing`. A match
    /// also needs the run's record held live by this app, whose in-memory ledger checks the same snapshot
    /// (`CoreRecord::check_snapshot`, no second read); a matching file with no held record is `NotHeld`, never `Match`.
    pub(super) fn ledger_of(&self, slug: &str, verified: &Value) -> proof::Ledger {
        use super::record::{LedgerStore, ReadBack};
        use crate::vajra::bridge::Request;
        #[cfg(test)]
        if let Some(l) = self.proof_ledger {
            return l;
        }
        let Ok(store) = self.ledger_store(slug, &self.bridge) else { return proof::Ledger::Mismatch };
        let ledger = match store.load() {
            Ok(Some(l)) if l.slug == slug => l,
            Ok(None) => return proof::Ledger::Missing,
            // Unreadable, or saved under another campaign's name: never a match.
            _ => return proof::Ledger::Mismatch,
        };
        let (Some(campaign), Some(features), Some(progress)) = (verified.get("campaign"), verified.get("features"), verified["progress"].as_str()) else {
            return proof::Ledger::Mismatch;
        };
        // A run still going reads In progress before the ledger counts: no task-graph read on every poll.
        if campaign["status"] == "active" {
            return proof::Ledger::Missing;
        }
        let tasks = match self.bridge.run(&Request::TasksList { slug: slug.into() }) {
            Ok(o) if o.exit() == Some(0) && !o.truncated => match serde_json::from_str::<Value>(&o.stdout) {
                Ok(v) => v,
                Err(_) => return proof::Ledger::Mismatch,
            },
            _ => return proof::Ledger::Mismatch,
        };
        let back = ReadBack { campaign: campaign.clone(), features: features.clone(), progress: progress.to_string(), tasks };
        if ledger.check(&back).is_err() {
            return proof::Ledger::Mismatch;
        }
        // The file alone is never a match (#226's rule): until key A an agent can write it and sign a campaign to fit.
        // Only a record this app still holds counts, and its in-memory ledger checks this same snapshot: a second read
        // could see the genuine files restored after an altered snapshot passed the file check (#237 review).
        let Some(rec) = self.live_record(slug) else { return proof::Ledger::NotHeld };
        if rec.check_snapshot(&back).is_err() {
            return proof::Ledger::Mismatch;
        }
        proof::Ledger::Match
    }

    /// The run's record, if the app still holds it, for this campaign.
    pub(super) fn live_record(&self, slug: &str) -> Option<std::sync::Arc<super::record::CoreRecord>> {
        use super::record::RunRecord;
        let run = slug.strip_prefix("deliver-").map(|r| r.strip_suffix("-og").unwrap_or(r))?;
        let rec = lock(&self.live_records).get(run)?.upgrade()?;
        (rec.card().campaign == slug).then_some(rec)
    }

    fn campaign_redactor(&self, slug: &str) -> Redactor {
        let mut red = Redactor::default();
        for r in lock(&self.runs).values().filter(|r| r.campaign == slug) {
            red.absorb(&r.nonces);
        }
        red
    }

    /// The self-contained page, written under the app data dir's `exports/` and returned with its path. Behind the one
    /// export capability check.
    pub fn export_proof_page(&self, slug: &str) -> Res {
        if !proof::export_allowed() {
            return Err(fail("export_unavailable", "exporting the proof is not available in this build"));
        }
        let p = self.gather_proof(slug)?;
        // The data dir and `exports/` are opened without following a symlink, and the write is relative to the held
        // folder: an `exports` link, or one swapped in after the open, never sends the page (or a chmod) elsewhere.
        use crate::projects::pinned::Pinned;
        let dir = self.data.join("exports");
        let held = Pinned::open(&self.data, false).and_then(|d| d.child("exports", true)).map_err(|e| fail("io_error", e))?;
        let name = format!("{slug}-proof.html");
        held.write_atomic(&name, proof::page(&p).as_bytes()).map_err(|e| fail("io_error", e))?;
        Ok(json!({"slug": slug, "path": dir.join(name).to_string_lossy(), "label": p.label.id()}))
    }

    /// Post the PR comment: a human click only, confirmed per run with the repo and PR named. The text is rebuilt here
    /// from the record, never taken from the UI, so what is posted is exactly what the app shows.
    pub fn act_post_proof_comment(&self, env: &Envelope, slug: &str, pr_url: &str) -> Value {
        let target = format!("proof:{slug}");
        self.act(env, crate::control::receipt::Caller::Ui, true, "post_proof_comment", &target, None::<u64>, || self.post_proof_comment(slug, pr_url))
    }

    fn post_proof_comment(&self, slug: &str, pr_url: &str) -> Res {
        if !proof::export_allowed() {
            return Err(fail("export_unavailable", "exporting the proof is not available in this build"));
        }
        let pr = proof::parse_pr(pr_url).ok_or_else(|| invalid("pr", "use a GitHub pull request link: https://github.com/<owner>/<repo>/pull/<number>"))?;
        let p = self.gather_proof(slug)?;
        if p.label == Label::InProgress {
            return Err(fail("run_not_finished", "the run has not finished, so there is no proof to post yet"));
        }
        let body = proof::markdown(&p);
        if body.len() > COMMENT_MAX {
            return Err(fail("comment_too_long", "the proof is too long for one comment; export the page instead"));
        }
        let gh = self.find_gh().ok_or_else(|| fail("gh_missing", "GitHub CLI (gh) was not found. Install it and sign in, then try again."))?;
        let argv: Vec<String> = ["pr", "comment", &pr.number.to_string(), "-R", &pr.repo, "--body-file", "-"].map(String::from).to_vec();
        let o = crate::vajra::bridge::run_process_full(&gh, &argv, None, POST_TIMEOUT, Some(&self.home), Some(body.as_bytes())).map_err(|e| fail("post_failed", e))?;
        let home = self.home.to_string_lossy();
        if o.exit() != Some(0) {
            let why = o.stderr.lines().find(|l| !l.trim().is_empty()).map(|l| crate::diagnostics::redact(&redact_provider_text(l.trim()), &home))
                .unwrap_or_else(|| "gh exited with an error".into());
            return Err(fail("post_failed", why));
        }
        Ok(json!({"slug": slug, "repo": pr.repo, "pr": pr.number, "label": p.label.id(), "url": o.stdout.trim().lines().last().map(|l| crate::diagnostics::redact(&redact_provider_text(l), &home))}))
    }

    fn find_gh(&self) -> Option<PathBuf> {
        let dirs = self.provider_dirs.clone().unwrap_or_else(crate::vajra::preflight::path_dirs);
        dirs.iter().map(|d| d.join("gh")).find(|p| is_exec(p))
    }
}

fn is_hex(s: &str, len: std::ops::RangeInclusive<usize>) -> bool {
    len.contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_exec(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
