//! Read-only export boundary for the isolated mobile spike. No App/control/socket handle enters this module.
use crate::projects::contract::{Project, Session, SessionKind};
use serde_json::{Value, json};

/// Export only explicitly scoped, validated records. Names are owner-approved display data, not paths/prompts.
/// Attention is a count per run; no question, plan, command, token or transcript crosses this boundary.
pub fn snapshot(
    projects: &[Project],
    sessions: &[Session],
    attention: &[(String, u32)],
    scope: &[String],
) -> Result<Value, &'static str> {
    if projects.len() > 64 || sessions.len() > 512 || attention.len() > 512 || scope.len() > 64 {
        return Err("snapshot_limit");
    }
    let mut rows = Vec::new();
    for p in projects
        .iter()
        .filter(|p| scope.contains(&p.id) && p.archived.is_none())
    {
        p.validate().map_err(|_| "invalid_project")?;
        let mut runs = Vec::new();
        for s in sessions
            .iter()
            .filter(|s| s.project_id == p.id && s.kind == SessionKind::Run)
        {
            s.validate().map_err(|_| "invalid_session")?;
            let count = attention
                .iter()
                .find(|(id, _)| id == &s.id)
                .map_or(0, |(_, n)| *n);
            if count > 100_000 {
                return Err("attention_limit");
            }
            runs.push(json!({"id": s.id, "status": s.status, "needs_you": count}));
        }
        // No paths, native session IDs, provider settings, text from runs, or action receipts.
        let name: String = p
            .name
            .chars()
            .filter(|c| !c.is_control())
            .take(80)
            .collect();
        rows.push(json!({"id": p.id, "name": name, "runs": runs}));
    }
    let result = json!({"schema": "vajra.mobile_status/1", "projects": rows});
    if result.to_string().len() > 64 * 1024 {
        return Err("snapshot_limit");
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::contract::{RootKey, make_id};
    #[test]
    fn projection_scopes_and_drops_private_fields() {
        let a = Project::new(
            make_id("p", 1, [1; 10]),
            "Allowed".into(),
            "/secret/root".into(),
            RootKey {
                dev: 1,
                ino: 2,
                git_common_dir: None,
            },
            "2026-11-08T00:00:00Z".into(),
        );
        let mut b = a.clone();
        b.id = make_id("p", 2, [2; 10]);
        b.name = "Excluded".into();
        let v = snapshot(&[a.clone(), b], &[], &[], &[a.id.clone()])
            .unwrap()
            .to_string();
        assert!(v.contains("Allowed"));
        assert!(!v.contains("Excluded"));
        assert!(!v.contains("secret"));
        assert!(!v.contains("root_key"));
        assert_eq!(
            snapshot(&[], &[], &[], &vec!["x".into(); 65]),
            Err("snapshot_limit")
        );
    }

    #[test]
    fn run_attention_is_status_only_and_archived_projects_are_excluded() {
        use crate::projects::contract::{SESSION_SCHEMA, SessionStatus};
        let mut p = Project::new(
            make_id("p", 1, [1; 10]),
            "Allowed".into(),
            "/secret/root".into(),
            RootKey {
                dev: 1,
                ino: 2,
                git_common_dir: None,
            },
            "2026-11-08T00:00:00Z".into(),
        );
        let run = Session {
            schema: SESSION_SCHEMA.into(),
            id: make_id("s", 1, [1; 10]),
            project_id: p.id.clone(),
            kind: SessionKind::Run,
            parent: None,
            provider: "claude".into(),
            native_session_id: None,
            run_id: Some("20261108-0900".into()),
            campaign: Some("private-campaign".into()),
            task_id: None,
            attempt: None,
            generation: 1,
            status: SessionStatus::Running,
            started: "2026-11-08T00:00:00Z".into(),
            ended: None,
        };
        let v = snapshot(
            &[p.clone()],
            &[run.clone()],
            &[(run.id.clone(), 2)],
            &[p.id.clone()],
        )
        .unwrap();
        assert_eq!(v["projects"][0]["runs"][0]["needs_you"], 2);
        assert_eq!(v["projects"][0]["runs"][0]["status"], "running");
        assert!(!v.to_string().contains("private-campaign"));
        assert!(!v.to_string().contains("claude"));
        assert!(
            snapshot(
                &[p.clone()],
                &[run.clone()],
                &[(run.id.clone(), 100001)],
                &[p.id.clone()]
            )
            .is_err()
        );
        p.archived = Some("2026-11-08T00:01:00Z".into());
        assert_eq!(
            snapshot(&[p.clone()], &[run], &[], &[p.id]).unwrap()["projects"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
    }
}
