//! The Lead-ask and plan-decision records (ADR-13, ADR-14): what the app writes to the ledger for a human question or approval.

use super::*;

impl Service {
    // ------------------------------------------------------------ asks

    /// Append one ask line. An ask is human and `ui` by construction. It writes no `audit.log` line (it acts on no
    /// worker), enters no queue, and is never pending, so the ack watcher and reconcile have nothing to do with it.
    /// The caller records `queued` before it spawns the Lead and stops if this fails.
    /// Test only: appends fail from now on.
    #[cfg(test)]
    pub(crate) fn break_ledger_for_test(&mut self) {
        self.ledger.break_for_test();
    }

    pub fn record_ask(&mut self, r: &AskRecord<'_>) -> Result<()> {
        let now = SystemTime::now();
        let mut a = self.new_action(Kind::Ask, Actor::Human, None, now)?;
        a.id = r.ask_id.to_string();
        a.run_id = r.run_id.to_string();
        a.session_id = r.session_id.to_string();
        a.reason = r.reason.map(str::to_string);
        let mut extra = json!({"lead_session_id": r.lead_session_id});
        if let Some((project, generation)) = r.project {
            extra["project_id"] = json!(project);
            extra["generation"] = json!(generation);
        }
        if let Some(tokens) = r.tokens {
            extra["tokens"] = tokens.clone();
        }
        if let Some(t) = r.text {
            extra["text"] = json!(t);
            extra["text_sha256"] = json!(sha256::hex_digest(t.as_bytes()));
        }
        self.record(&a, r.state.state(), extra, now)
    }

    /// The action that already decided plan `item`, if any, whatever entry it named: an approve or reject whose newest line is `requested`,
    /// `delivered` or `done`. A `requested` with no outcome (a crash mid-delivery) counts as decided: delivery is
    /// unknown, so the plan is never run twice. Only `failed` frees the item.
    ///
    /// Keyed on the id alone (ADR-14 §5: one terminal decision per plan id). The turn an approval delivers names only
    /// `<run_id>.<index>`, so a second decision on the same id after its entry changed would reach the session as the
    /// same turn, which it may map to the entry the human rejected or already approved.
    pub fn decided(&self, item: &str) -> io::Result<Option<String>> {
        let records = Ledger::read_all(&self.control_root.join("ledger.jsonl"))?;
        let mut last: Vec<(String, String)> = Vec::new();
        for r in records.iter().filter(|r| matches!(str_field(r, "kind"), "approve" | "reject") && str_field(r, "item") == item) {
            let (id, state) = (str_field(r, "action_id").to_string(), str_field(r, "state").to_string());
            match last.iter_mut().find(|(i, _)| *i == id) {
                Some(e) => e.1 = state,
                None => last.push((id, state)),
            }
        }
        Ok(last.into_iter().find(|(_, st)| st != "failed").map(|(id, _)| id))
    }

    /// Append one decision line and return the action id. The final line also appends the `audit.log` line
    /// (`approve id=<action_id> item=<item>` or `reject ...`) that reconcile joins on. A failed audit append is logged,
    /// not hidden: reconcile reports the action as a ledger line without an audit line.
    pub fn record_decision(&mut self, r: &DecisionRecord<'_>) -> Result<String> {
        let now = SystemTime::now();
        let kind = if r.decision == Decision::Approve { Kind::Approve } else { Kind::Reject };
        let mut a = self.new_action(kind, Actor::Human, None, now)?;
        if let Some(id) = r.action_id {
            a.id = id.to_string();
        }
        a.run_id = r.run_id.to_string();
        a.session_id = r.session_id.to_string();
        a.reason = r.reason.map(str::to_string);
        let mut extra = json!({"item": r.item, "campaign": r.campaign});
        if let Some(h) = r.plan_sha256 {
            extra["plan_sha256"] = json!(h);
        }
        let (state, verb) = match (r.state, r.decision) {
            (DecisionState::Requested, _) => (State::Requested, ""),
            (DecisionState::Failed, _) => (State::Failed, ""),
            (DecisionState::Final, Decision::Approve) => (State::Delivered, "approve"),
            (DecisionState::Final, Decision::Reject) => (State::Done, "reject"),
        };
        self.record(&a, state, extra, now)?;
        if !verb.is_empty() && audit::append_line(&self.audit_log, "ALLOW", &format!("{verb} id={} item={}", a.id, r.item), now).is_err() {
            self.log("control_audit_failed", &[]);
        }
        Ok(a.id)
    }
}
