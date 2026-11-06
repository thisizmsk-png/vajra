//! C5 crew module: tailer, record parsing, nonce redaction, names and the
//! crew tree. Everything runs against fixtures copied into a temp HOME.

mod crew {
    use std::collections::HashSet;
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::json;
    use vajra_desktop::bus::{Bus, Config};
    use vajra_desktop::crew::names::{title_case, ui_name};
    use vajra_desktop::crew::record::{LineKind, parse_journal, parse_meta};
    use vajra_desktop::crew::redact::Redactor;
    use vajra_desktop::crew::tailer::{Tailer, Transcript};
    use vajra_desktop::crew::tree::{self, CrewInput, Node, NodeKind, Signals, Status, Worker};
    use vajra_desktop::vajra::allowlist::Allowlist;
    use vajra_desktop::vajra::state::Reader;

    static N: AtomicUsize = AtomicUsize::new(0);

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/transcripts")
    }

    fn copy_dir(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for e in fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            let dest = to.join(e.file_name());
            if e.file_type().unwrap().is_dir() {
                copy_dir(&e.path(), &dest);
            } else {
                fs::copy(e.path(), dest).unwrap();
            }
        }
    }

    /// A temp HOME with `~/.claude/projects/-repo/<session>/...` holding the
    /// transcript fixtures and `~/.claude/vajra/campaigns/<slug>/tasks.json`.
    struct Home {
        home: PathBuf,
    }

    const SLUG: &str = "deliver-20261105-1810";
    const RUN: &str = "20261105-1810";

    impl Home {
        fn new() -> Self {
            let home = std::env::temp_dir()
                .join(format!("vajra-crew-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
            let _ = fs::remove_dir_all(&home);
            copy_dir(&fixtures(), &home.join(".claude/projects/-repo"));
            fs::create_dir_all(home.join(".claude/vajra/campaigns").join(SLUG)).unwrap();
            fs::create_dir_all(home.join("appdata")).unwrap();
            Self { home }
        }
        fn project(&self) -> PathBuf {
            self.home.join(".claude/projects/-repo")
        }
        fn allow(&self) -> Allowlist {
            Allowlist::new(&self.home.join(".claude"), &self.home.join("appdata")).unwrap()
        }
        fn tasks_path(&self) -> PathBuf {
            self.home.join(".claude/vajra/campaigns").join(SLUG).join("tasks.json")
        }
        fn write_tasks(&self, statuses: &[(&str, &str, &str)]) {
            let tasks: Vec<_> = statuses
                .iter()
                .map(|(id, owner, status)| json!({"id": id, "owner_agent": owner, "status": status, "claimed_by": owner}))
                .collect();
            let doc = json!({"campaign": SLUG, "schema": "vajra.tasks/v1", "tasks": tasks});
            fs::write(self.tasks_path(), serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
        }
    }

    impl Drop for Home {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.home);
        }
    }

    fn append(path: &Path, bytes: &[u8]) {
        let mut f = OpenOptions::new().append(true).create(true).open(path).unwrap();
        f.write_all(bytes).unwrap();
    }

    fn find<'a>(node: &'a Node, title: &str) -> Option<&'a Node> {
        if node.title == title {
            return Some(node);
        }
        node.children.iter().find_map(|c| find(c, title))
    }

    fn all_titles(node: &Node, out: &mut Vec<String>) {
        out.push(node.title.clone());
        for c in &node.children {
            all_titles(c, out);
        }
    }

    fn worker(id: &str, engine: &str, label: &str, status: Status) -> Worker {
        Worker {
            agent_id: id.into(),
            engine_id: engine.into(),
            label: label.into(),
            prompt_first_line: None,
            branch: None,
            status,
        }
    }

    // ---- tailer ----

    #[test]
    fn tailer_holds_partial_last_line_until_newline() {
        let h = Home::new();
        let p = h.project().join("live.jsonl");
        fs::write(&p, b"one\ntw").unwrap();
        let mut t = Tailer::new(h.allow(), p.clone());
        let c = t.poll().unwrap();
        assert_eq!(c.lines, vec!["one".to_string()]);
        assert!(!c.truncated);
        append(&p, b"o\nthree");
        assert_eq!(t.poll().unwrap().lines, vec!["two".to_string()]);
        assert!(t.poll().unwrap().lines.is_empty(), "no newline yet, nothing new");
        append(&p, b"\n");
        assert_eq!(t.poll().unwrap().lines, vec!["three".to_string()]);
    }

    #[test]
    fn tailer_skips_an_oversized_record_with_a_placeholder() {
        use vajra_desktop::crew::tailer::{OVERSIZED, PARTIAL_CAP, READ_CAP};
        const { assert!(PARTIAL_CAP <= 1024 * 1024 && READ_CAP <= 1024 * 1024, "per-worker budget") };
        let h = Home::new();
        let p = h.project().join("big.jsonl");
        let mut big = vec![b'x'; PARTIAL_CAP * 3];
        big.push(b'\n');
        fs::write(&p, b"one\n").unwrap();
        append(&p, &big);
        append(&p, b"two\n");
        let mut t = Tailer::new(h.allow(), p.clone());
        let mut got = Vec::new();
        for _ in 0..16 {
            got.extend(t.poll().unwrap().lines);
        }
        assert_eq!(got, vec!["one".to_string(), OVERSIZED.to_string(), "two".to_string()]);
    }

    #[test]
    fn oversized_record_keeps_the_transcript_recognized() {
        use vajra_desktop::crew::tailer::PARTIAL_CAP;
        let h = Home::new();
        let p = h.project().join("big2.jsonl");
        let rec = json!({"type": "user", "message": {"role": "user", "content": "x".repeat(PARTIAL_CAP * 2)}});
        fs::write(&p, format!("{rec}\n")).unwrap();
        let mut tr = Transcript::new(Tailer::new(h.allow(), p), Redactor::new(None));
        for _ in 0..8 {
            tr.poll().unwrap();
        }
        assert!(!tr.format_unrecognized());
    }

    #[test]
    fn tailer_restarts_from_zero_after_truncation() {
        let h = Home::new();
        let p = h.project().join("live.jsonl");
        fs::write(&p, b"aaaa\nbbbb\npart").unwrap();
        let mut t = Tailer::new(h.allow(), p.clone());
        assert_eq!(t.poll().unwrap().lines.len(), 2);
        fs::write(&p, b"new\n").unwrap();
        let c = t.poll().unwrap();
        assert!(c.truncated, "shorter file is reported as truncated");
        assert_eq!(c.lines, vec!["new".to_string()], "stale partial dropped, new content read");
    }

    #[test]
    fn tailer_treats_replaced_file_as_truncation() {
        let h = Home::new();
        let p = h.project().join("live.jsonl");
        fs::write(&p, b"first\n").unwrap();
        let mut t = Tailer::new(h.allow(), p.clone());
        t.poll().unwrap();
        let tmp = h.project().join("live.tmp");
        fs::write(&tmp, b"second file longer\nmore\n").unwrap();
        fs::rename(&tmp, &p).unwrap();
        let c = t.poll().unwrap();
        assert!(c.truncated);
        assert_eq!(c.lines, vec!["second file longer".to_string(), "more".to_string()]);
    }

    #[test]
    fn tailer_missing_file_is_empty_not_an_error() {
        let h = Home::new();
        let mut t = Tailer::new(h.allow(), h.project().join("not-yet.jsonl"));
        let c = t.poll().unwrap();
        assert!(c.lines.is_empty());
    }

    #[test]
    fn tailer_refuses_paths_outside_the_allowlist() {
        let h = Home::new();
        let outside = h.home.join("elsewhere.jsonl");
        fs::write(&outside, b"secret\n").unwrap();
        let mut t = Tailer::new(h.allow(), outside);
        assert!(t.poll().is_err());
    }

    // ---- record parsing ----

    #[test]
    fn crew_fixture_transcript_parses_without_format_flag() {
        let h = Home::new();
        let p = h.project().join("session/subagents/agent-aa870dd40540481f0.jsonl");
        let mut tr = Transcript::new(Tailer::new(h.allow(), p), Redactor::new(None));
        let lines = tr.poll().unwrap();
        assert!(!tr.format_unrecognized());
        assert!(lines.iter().any(|l| l.kind == LineKind::Tool && l.text.contains("echo step1")));
        assert!(lines.iter().any(|l| l.kind == LineKind::ToolResult));
        assert!(lines.iter().any(|l| l.kind == LineKind::Hook));
        assert!(lines.iter().all(|l| l.kind != LineKind::Raw));
        assert!(tr.prompt_first_line().unwrap().starts_with("Run these Bash commands"));
    }

    #[test]
    fn crew_meta_and_journal_fixtures_parse() {
        let wf = fixtures().join("session/subagents/workflows/wf_fixture");
        let meta = parse_meta(&fs::read_to_string(wf.join("agent-a3dea76dc37f6f3e1.meta.json")).unwrap()).unwrap();
        assert_eq!(meta.agent_type, "abhimanyu");
        assert_eq!(meta.description, "kb.search");
        let j = parse_journal(&fs::read_to_string(wf.join("journal.jsonl")).unwrap());
        let kb = j.iter().find(|e| e.agent_id == "a3dea76dc37f6f3e1").unwrap();
        assert_eq!(kb.label, "kb.search");
        assert!(kb.finished);
        let plan2 = j.iter().find(|e| e.agent_id == "a33208c29ba12a5f3").unwrap();
        assert!(!plan2.finished, "started with no result is still running");
        assert_eq!(j.len(), 5);
    }

    #[test]
    fn crew_unknown_record_shape_sets_flag_and_falls_back_to_raw() {
        let h = Home::new();
        let p = h.project().join("shapes/unknown-shape.jsonl");
        let mut tr = Transcript::new(Tailer::new(h.allow(), p), Redactor::new(None));
        let lines = tr.poll().unwrap();
        assert!(tr.format_unrecognized());
        assert_eq!(tr.banner(), Some("Transcript format not recognized; showing raw view"));
        assert!(lines.iter().any(|l| l.kind == LineKind::Text && l.text == "structured line"));
        let raw: Vec<_> = lines.iter().filter(|l| l.kind == LineKind::Raw).collect();
        assert_eq!(raw.len(), 2, "the odd record and everything after it is raw");
        assert!(raw[0].text.contains("never wrote"));
        assert!(raw[1].text.contains("after the change"));
        assert_eq!(tr.git_branch(), Some("vajra/20261105-1810/t1"));
        assert_eq!(
            tr.prompt_first_line(),
            Some("You own task t1 (task graph id 20261105-1810.t1) of a larger delivery.")
        );
    }

    #[test]
    fn crew_non_json_line_sets_format_flag() {
        let h = Home::new();
        let p = h.project().join("live.jsonl");
        fs::write(&p, b"this is not json\n").unwrap();
        let mut tr = Transcript::new(Tailer::new(h.allow(), p), Redactor::new(None));
        let lines = tr.poll().unwrap();
        assert!(tr.format_unrecognized());
        assert_eq!(lines[0].kind, LineKind::Raw);
        assert_eq!(lines[0].text, "this is not json");
    }

    // ---- nonce redaction ----

    const NONCE: &str = "N0nce-Fixture-0123456789";

    #[test]
    fn crew_nonce_is_redacted_from_every_stream_line() {
        let h = Home::new();
        let p = h.project().join("shapes/steer.jsonl");
        let mut tr = Transcript::new(Tailer::new(h.allow(), p), Redactor::new(Some(NONCE)));
        let lines = tr.poll().unwrap();
        assert!(!tr.format_unrecognized());
        assert!(lines.len() >= 4);
        for l in &lines {
            assert!(!l.text.contains(NONCE), "nonce leaked: {:?}", l.text);
            assert!(!l.text.contains("0123456789"), "split nonce leaked: {:?}", l.text);
            assert!(!l.text.contains('\u{1b}'), "escape kept: {:?}", l.text);
            assert!(!l.text.contains('\u{200b}'), "zero-width kept: {:?}", l.text);
        }
        assert!(lines.iter().any(|l| l.text.contains("[redacted]")));
        assert!(tr.prompt_first_line().unwrap().contains("[redacted]"));
    }

    #[test]
    fn crew_real_fixture_nonce_is_redacted_when_pumped_to_bus() {
        let h = Home::new();
        let p = h.project().join("session/subagents/agent-aa870dd40540481f0.jsonl");
        let fixture_nonce = "c84b80d347ff2b24";
        assert!(fs::read_to_string(&p).unwrap().contains(fixture_nonce), "fixture carries the nonce");
        let bus = Bus::open(&h.home.join("appdata/spill"), Config::default()).unwrap();
        let mut tr = Transcript::new(Tailer::new(h.allow(), p), Redactor::new(Some(fixture_nonce)));
        let pushed = tr.pump(&bus, "agent-aa870dd40540481f0").unwrap();
        assert!(pushed > 0);
        let tail = bus.tail("agent-aa870dd40540481f0", 0, 1 << 20);
        assert_eq!(tail.lines.len(), pushed);
        assert!(tail.lines.iter().all(|l| !l.text.contains(fixture_nonce)));
        assert!(tail.lines.iter().any(|l| l.text.contains("[redacted]")));
    }

    #[test]
    fn crew_redactor_ignores_short_or_empty_nonce() {
        assert_eq!(Redactor::new(Some("")).clean("a b"), "a b");
        assert_eq!(Redactor::new(Some("ab")).clean("ab cab"), "ab cab");
    }

    // ---- names ----

    #[test]
    fn crew_names_map_known_ids_and_title_case_unknown() {
        assert_eq!(ui_name("krishna"), "Lead");
        assert_eq!(ui_name("bhima"), "Backend");
        assert_eq!(ui_name("duryodhana"), "Critic (pre-mortem)");
        assert_eq!(ui_name("sepoy-frontend-1"), "Frontend worker 1");
        assert_eq!(ui_name("foo-bar"), "Foo Bar");
        assert_eq!(ui_name("general-purpose"), "General Purpose");
        assert_eq!(title_case("kb.search"), "Kb Search");
        assert_eq!(title_case("deploy_plan"), "Deploy Plan");
    }

    // ---- tree ----

    #[test]
    fn crew_tree_matches_run_prefixed_graph_ids() {
        let tasks = json!({"tasks": [{"id": format!("{RUN}.t1"), "owner_agent": "bhima", "status": "in_progress"}]});
        let workers = vec![worker("a1", "bhima", "task.t1.r1", Status::InProgress)];
        let signals = Signals::default();
        let root = tree::build(&CrewInput {
            run_id: RUN, lead: None, run_status: Status::InProgress, tasks: Some(&tasks), workers: &workers, signals: &signals,
        });
        let all = format!("{root:?}");
        assert_eq!(all.matches("kind: Task").count(), 1, "one task node, not a status-less duplicate: {all}");
        assert!(all.contains("in_progress") || all.contains("InProgress"));
    }

    #[test]
    fn crew_tree_shape_lead_run_tasks_rounds_verification_run_steps() {
        let tasks = json!({"tasks": [
            {"id": "t1", "owner_agent": "bhima", "status": "in_progress"},
            {"id": "t2", "owner_agent": "nakula", "status": "pending"},
        ]});
        let workers = vec![
            worker("a1", "bhima", "task.t1.r1", Status::Done),
            worker("a2", "bhima", "task.t1.r2", Status::InProgress),
            worker("a3", "vidura", "verify.r1", Status::Done),
            worker("a4", "vidura", "verify.r1.commands", Status::Done),
            worker("a5", "bhishma", "review.bhishma.r1", Status::Done),
            worker("a6", "vidura", "writeback", Status::Unknown),
            worker("a7", "abhimanyu", "kb.search", Status::Done),
            worker("a8", "abhimanyu", "campaign.open", Status::Done),
            worker("a9", "abhimanyu", "mystery.step", Status::Unknown),
        ];
        let signals = Signals::default();
        let root = tree::build(&CrewInput {
            run_id: RUN,
            lead: Some(Status::InProgress),
            run_status: Status::InProgress,
            tasks: Some(&tasks),
            workers: &workers,
            signals: &signals,
        });
        assert_eq!(root.kind, NodeKind::Lead);
        assert_eq!(root.title, "Lead");
        assert_eq!(root.children.len(), 1);
        let run = &root.children[0];
        assert_eq!(run.kind, NodeKind::Run);
        assert_eq!(run.title, format!("Run {RUN}"));
        let kinds: Vec<_> = run.children.iter().map(|c| (c.kind, c.title.as_str())).collect();
        assert_eq!(
            kinds,
            vec![
                (NodeKind::Task, "Backend · t1"),
                (NodeKind::Task, "Frontend · t2"),
                (NodeKind::Verification, "Verification r1"),
                (NodeKind::WriteBack, "Write-back"),
                (NodeKind::RunSteps, "Run steps"),
            ]
        );
        let t1 = &run.children[0];
        assert_eq!(t1.status, Status::InProgress);
        let rounds: Vec<_> = t1.children.iter().map(|c| (c.kind, c.title.as_str(), c.status)).collect();
        assert_eq!(
            rounds,
            vec![(NodeKind::Round, "round 1", Status::Done), (NodeKind::Round, "round 2", Status::InProgress)]
        );
        assert_eq!(t1.children[0].engine_id.as_deref(), Some("bhima"));
        assert!(run.children[1].children.is_empty(), "t2 exists from tasks.json before any transcript");
        assert_eq!(run.children[1].status, Status::Pending);

        let v = &run.children[2];
        let vt: Vec<_> = v.children.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(vt, vec!["Verifier", "Verifier", "Security review"]);
        let steps: Vec<_> = run.children[4].children.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(steps, vec!["Kb Search", "Campaign Open", "Mystery Step"]);

        let mut titles = Vec::new();
        all_titles(&root, &mut titles);
        for engine in ["bhima", "nakula", "vidura", "bhishma", "abhimanyu", "krishna"] {
            assert!(titles.iter().all(|t| !t.to_lowercase().contains(engine)), "{engine} in {titles:?}");
        }
    }

    #[test]
    fn crew_tree_matches_by_prompt_then_branch_then_unassigned() {
        let tasks = json!({"tasks": [
            {"id": "t1", "owner_agent": "bhima", "status": "claimed"},
            {"id": "t2", "owner_agent": "nakula", "status": "claimed"},
        ]});
        let mut by_prompt = worker("p1", "bhima", "", Status::InProgress);
        by_prompt.prompt_first_line = Some("You own task t1 (task graph id 20261105-1810.t1) of a larger delivery.".into());
        let mut by_branch = worker("b1", "nakula", "", Status::InProgress);
        by_branch.branch = Some(format!("vajra/{RUN}/t2-r2"));
        let lost = worker("x1", "sepoy-test-1", "", Status::Unknown);
        let workers = vec![by_prompt, by_branch, lost];
        let root = tree::build(&CrewInput {
            run_id: RUN,
            lead: None,
            run_status: Status::Unknown,
            tasks: Some(&tasks),
            workers: &workers,
            signals: &Signals::default(),
        });
        assert_eq!(root.status, Status::Unknown, "no Lead session is 'unknown', never idle");
        let run = &root.children[0];
        assert_eq!(find(run, "Backend · t1").unwrap().children[0].id, "agent:p1");
        let t2 = find(run, "Frontend · t2").unwrap();
        assert_eq!(t2.children[0].title, "round 2");
        let un = find(run, "Unassigned").unwrap();
        assert_eq!(un.children[0].title, "Test worker 1");
    }

    #[test]
    fn crew_two_workers_same_engine_and_round_are_separate_nodes() {
        let tasks = json!({"tasks": [{"id": "t1", "owner_agent": "bhima", "status": "in_progress"}]});
        let workers = vec![
            worker("a1", "bhima", "task.t1.r1", Status::InProgress),
            worker("a2", "bhima", "task.t1.r1", Status::InProgress),
        ];
        let root = tree::build(&CrewInput {
            run_id: RUN,
            lead: None,
            run_status: Status::InProgress,
            tasks: Some(&tasks),
            workers: &workers,
            signals: &Signals::default(),
        });
        let t1 = find(&root, "Backend · t1").unwrap();
        let ids: Vec<_> = t1.children.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["agent:a1", "agent:a2"]);
    }

    #[test]
    fn crew_status_unknown_when_no_signal() {
        let tasks = json!({"tasks": [
            {"id": "t1", "owner_agent": "bhima"},
            {"id": "t2", "owner_agent": "bhima", "status": "sleeping"},
        ]});
        let workers = vec![worker("a1", "bhima", "task.t9.r1", Status::Unknown)];
        let root = tree::build(&CrewInput {
            run_id: RUN,
            lead: None,
            run_status: Status::Unknown,
            tasks: Some(&tasks),
            workers: &workers,
            signals: &Signals::default(),
        });
        assert_eq!(find(&root, "Backend · t1").unwrap().status, Status::Unknown);
        assert_eq!(find(&root, "Backend · t2").unwrap().status, Status::Unknown);
        let t9 = find(&root, "Backend · t9").unwrap();
        assert_eq!(t9.status, Status::Unknown, "task only seen in a label has no task status");
        assert_eq!(t9.children[0].status.as_str(), "unknown");

        let none = tree::build(&CrewInput {
            run_id: RUN,
            lead: None,
            run_status: Status::Unknown,
            tasks: None,
            workers: &[],
            signals: &Signals::default(),
        });
        assert_eq!(none.children[0].status, Status::Unknown);
        assert!(none.children[0].children.is_empty());
    }

    #[test]
    fn crew_overlays_attach_to_worker_nodes() {
        let tasks = json!({"tasks": [{"id": "t1", "owner_agent": "bhima", "status": "in_progress"}]});
        let workers = vec![worker("a1", "bhima", "task.t1.r1", Status::InProgress)];
        let signals = Signals {
            stopped: HashSet::from(["a1".to_string()]),
            message_pending: HashSet::from(["a1".to_string()]),
            activity: HashSet::from(["a1".to_string()]),
        };
        let root = tree::build(&CrewInput {
            run_id: RUN,
            lead: None,
            run_status: Status::InProgress,
            tasks: Some(&tasks),
            workers: &workers,
            signals: &signals,
        });
        let r1 = find(&root, "round 1").unwrap();
        assert_eq!(r1.overlays, vec!["stopped by the human", "message pending", "activity"]);
    }

    #[test]
    fn crew_statuses_update_when_tasks_json_changes() {
        let h = Home::new();
        let mut reader = Reader::new(h.allow());
        let workers = vec![worker("a1", "bhima", "task.t1.r1", Status::InProgress)];
        let signals = Signals::default();

        h.write_tasks(&[("t1", "bhima", "claimed"), ("t2", "nakula", "ready")]);
        let a = tree::snapshot(&mut reader, SLUG, RUN, None, Status::InProgress, &workers, &signals);
        assert_eq!(find(&a, "Backend · t1").unwrap().status, Status::Claimed);
        assert_eq!(find(&a, "Frontend · t2").unwrap().status, Status::Ready);

        h.write_tasks(&[("t1", "bhima", "done"), ("t2", "nakula", "blocked")]);
        let b = tree::snapshot(&mut reader, SLUG, RUN, None, Status::InProgress, &workers, &signals);
        assert_eq!(find(&b, "Backend · t1").unwrap().status, Status::Done);
        assert_eq!(find(&b, "Frontend · t2").unwrap().status, Status::Blocked);
        assert_eq!(find(&b, "Frontend · t2").unwrap().status.as_str(), "blocked");

        fs::write(h.tasks_path(), b"{ half written").unwrap();
        let c = tree::snapshot(&mut reader, SLUG, RUN, None, Status::InProgress, &workers, &signals);
        assert_eq!(find(&c, "Backend · t1").unwrap().status, Status::Done, "keeps last good snapshot");
    }

    #[test]
    fn crew_workers_from_fixture_files_land_in_run_steps() {
        let h = Home::new();
        let wf = h.project().join("session/subagents/workflows/wf_fixture");
        let journal = parse_journal(&fs::read_to_string(wf.join("journal.jsonl")).unwrap());
        let mut workers = Vec::new();
        for e in &journal {
            let meta = parse_meta(&fs::read_to_string(wf.join(format!("agent-{}.meta.json", e.agent_id))).unwrap()).unwrap();
            workers.push(tree::worker_from(&e.agent_id, &meta, Some(e), None));
        }
        let root = tree::build(&CrewInput {
            run_id: RUN,
            lead: None,
            run_status: Status::InProgress,
            tasks: None,
            workers: &workers,
            signals: &Signals::default(),
        });
        let steps = find(&root, "Run steps").unwrap();
        let titles: Vec<_> = steps.children.iter().map(|c| (c.title.as_str(), c.status)).collect();
        assert_eq!(
            titles,
            vec![
                ("Kb Search", Status::Done),
                ("Understand", Status::Done),
                ("Preflight", Status::Done),
                ("Plan 1", Status::Done),
                ("Plan 2", Status::InProgress),
            ]
        );
        assert_eq!(steps.children[0].engine_id.as_deref(), Some("abhimanyu"));
    }
}
