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
    /// request). The service has one control session, so the run id in the prompt proves nothing: the call is
    /// refused unless that run is the current session's run, checked here on every call under the service lock.
    /// A Lead asked about a finished run, or one that outlives its run, can never see or steer another run's workers.
    pub fn call_tool_for_run(&mut self, run: Option<&str>, tool: &str, args: &Value) -> Result<Value> {
        if let Some(r) = run {
            if !crate::ids::valid_run_id(r) {
                return Err(ControlError::InvalidArgs("run".into()));
            }
            if self.session.as_ref().is_none_or(|s| s.run_id != r) {
                return Err(ControlError::RunNotCurrent);
            }
        }
        self.last_recorded = None;
        let out = self.dispatch_tool(tool, args)?;
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
            _ => self.stop_worker(lead, s("worker", true)?.unwrap_or(""), s("reason", false)?),
        }
    }
}
