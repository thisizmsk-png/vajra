//! The Lead's tool surface: the socket tool calls, argument checks against each tool's schema, and the dispatch to the actions.

use super::*;

impl Service {
    // ------------------------------------------------------------ the Lead's tool surface

    /// A tool call from the socket. The actor is always the Lead; arguments
    /// are checked against the tool's schema (no extra properties).
    ///
    /// Each successful call also appends one id-carrying audit line, `ALLOW desktop-control reason="mcp <tool>
    /// id=<action_id>"`: evidence, not protection (ADR-13 6). Reconcile joins the Lead's calls by that id, and a
    /// call with no guard line beside it means the Lead ran unguarded.
    pub fn call_tool(&mut self, tool: &str, args: &Value) -> Result<Value> {
        self.call_tool_for_run(None, tool, args)
    }

    /// `call_tool` for a Lead that was started for one run (`VAJRA_LEAD_RUN` in its MCP config, sent with every
    /// request). The run id in the prompt proves nothing: the call is refused unless that run is a live session, and
    /// it then sees and steers **only that session's** workers, checked here on every call under the service lock.
    /// A Lead asked about a finished run, or one that outlives its run, can never see or steer another run's workers.
    /// An unbound call is served only while exactly one session is live: with two it could not say which run it means.
    pub fn call_tool_for_run(&mut self, run: Option<&str>, tool: &str, args: &Value) -> Result<Value> {
        let bound = match run {
            Some(r) => {
                if !crate::ids::valid_run_id(r) {
                    return Err(ControlError::InvalidArgs("run".into()));
                }
                let s = self.sessions.values().find(|s| s.run_id == r).ok_or(ControlError::RunNotCurrent)?;
                Some(vec![s.id.clone()])
            }
            None if self.sessions.len() > 1 => return Err(ControlError::RunNotCurrent),
            None => None,
        };
        self.call_scoped(bound, tool, args)
    }

    /// `call_tool` for a project's Director-General (ADR-15 2.3): `VAJRA_LEAD_PROJECT` and `VAJRA_LEAD_GEN` in its MCP
    /// config, sent with every request. The call is refused unless `generation` is the project's current one (a process
    /// from a replaced conversation, or one the app never started, gets nothing), and it then sees and steers **only
    /// this project's** live run sessions. With none live it sees no workers; it never falls back to "every session".
    pub fn call_tool_for_project(&mut self, project: &str, generation: u32, tool: &str, args: &Value) -> Result<Value> {
        if !crate::ids::valid_project_ref(project) {
            return Err(ControlError::InvalidArgs("project".into()));
        }
        if let Some(gate) = &self.project_gate {
            gate(project)?;
        }
        if self.generations.get(project) != Some(&generation) {
            return Err(ControlError::StaleLeadGeneration);
        }
        let bound: Vec<String> = self.sessions.values().filter(|s| s.project.as_deref() == Some(project)).map(|s| s.id.clone()).collect();
        let prev = self.bound_project.replace((project.to_string(), generation));
        let out = self.call_scoped(Some(bound), tool, args);
        self.bound_project = prev;
        out
    }

    fn call_scoped(&mut self, bound: Option<Vec<String>>, tool: &str, args: &Value) -> Result<Value> {
        self.last_recorded = None;
        let prev = std::mem::replace(&mut self.scope, bound);
        let out = self.dispatch_tool(tool, args);
        self.scope = prev;
        let out = out?;
        let id = out.get("action_id").and_then(Value::as_str).map(str::to_string).or_else(|| self.last_recorded.take());
        if let Some(id) = id {
            if audit::append_line(&self.audit_log, "ALLOW", &format!("mcp {tool} id={id}"), SystemTime::now()).is_err() {
                self.log("control_audit_failed", &[]);
            }
        }
        Ok(out)
    }

    pub(super) fn dispatch_tool(&mut self, tool: &str, args: &Value) -> Result<Value> {
        let empty = Map::new();
        let args = match args {
            Value::Object(m) => m,
            Value::Null => &empty,
            _ => return Err(ControlError::InvalidArgs("arguments must be an object".into())),
        };
        let allowed: &[&str] = match tool {
            "list_workers" => &["run_id"],
            "inspect_worker" => &["worker", "lines"],
            "message_worker" | "redirect_worker" => &["worker", "text"],
            "stop_worker" => &["worker", "reason"],
            "propose_run" => &["ask", "base", "rounds"],
            _ => return Err(ControlError::UnknownTool),
        };
        if let Some(k) = args.keys().find(|k| !allowed.contains(&k.as_str())) {
            return Err(ControlError::InvalidArgs(format!("unknown property {k}")));
        }
        let s = |k: &str, req: bool| -> Result<Option<&str>> {
            match args.get(k) {
                None if !req => Ok(None),
                Some(Value::String(v)) => Ok(Some(v.as_str())),
                _ => Err(ControlError::InvalidArgs(format!("{k} must be a string"))),
            }
        };
        let lead = Actor::Lead;
        match tool {
            "list_workers" => {
                let run = s("run_id", false)?;
                if let Some(r) = run {
                    if !crate::ids::valid_run_id(r) {
                        return Err(ControlError::InvalidArgs("run_id".into()));
                    }
                }
                self.list_workers(lead, run)
            }
            "inspect_worker" => {
                let lines = match args.get("lines") {
                    None => 50,
                    Some(v) => v.as_u64().ok_or_else(|| ControlError::InvalidArgs("lines must be an integer".into()))?,
                };
                self.inspect_worker(lead, s("worker", true)?.unwrap_or(""), lines)
            }
            "message_worker" => self.message_worker(lead, s("worker", true)?.unwrap_or(""), s("text", true)?.unwrap_or("")),
            "redirect_worker" => self.redirect_worker(lead, s("worker", true)?.unwrap_or(""), s("text", true)?.unwrap_or("")),
            "propose_run" => {
                let rounds = match args.get("rounds") {
                    None => None,
                    Some(v) => Some(v.as_u64().ok_or_else(|| ControlError::InvalidArgs("rounds must be an integer".into()))?),
                };
                self.propose_run(s("ask", true)?.unwrap_or(""), s("base", false)?, rounds)
            }
            _ => self.stop_worker(lead, s("worker", true)?.unwrap_or(""), s("reason", false)?),
        }
    }
}
