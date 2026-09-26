//! SQL tables have one owner, checked against the migration history.

use crate::workspace::Workspace;
use crate::{Violation, annotations, sql};
use anyhow::Result;
use std::collections::BTreeSet;

pub const RULE: &str = "table-ownership";
/// Table ownership after applying all core migrations.
pub const TABLES: &[(&str, &str)] = &[
    ("adapter_app_audit_events", "moatless-identity"),
    ("adapter_app_migrations", "moatless-identity"),
    ("adapter_app_secrets", "moatless-identity"),
    ("adapter_apps", "moatless-identity"),
    ("adapter_connections", "moatless-adapter-model"),
    ("adapter_delivery", "moatless-loops"),
    ("adapter_event_receipts", "moatless-loops"),
    ("adapter_task_creation_claims", "moatless-loops"),
    ("api_keys", "moatless-identity"),
    ("auth_audit_events", "moatless-identity"),
    ("auth_rate_limits", "moatless-identity"),
    ("auth_sessions", "moatless-identity"),
    ("auth_transactions", "moatless-identity"),
    ("device_pairing_codes", "moatless-identity"),
    ("event_ingress_outbox", "moatless-tasks"),
    ("feedback", "moatless-tasks"),
    ("file_metadata", "moatless-core"),
    ("forgejo_instances", "moatless-identity"),
    ("github_app_audit_events", "moatless-identity"),
    ("github_app_migrations", "moatless-identity"),
    ("github_apps", "moatless-identity"),
    ("inbound_message_buffer", "moatless-loops"),
    ("integration_pairing_requests", "moatless-adapter-telegram"),
    ("loop_configs", "moatless-loops"),
    ("loops", "moatless-loops"),
    ("oidc_identities", "moatless-identity"),
    ("plugin_activations", "moatless-workspaces"),
    ("plugin_skills", "moatless-workspaces"),
    ("plugins", "moatless-workspaces"),
    ("preview_tab_changes", "moatless-core"),
    ("preview_tab_position", "moatless-core"),
    ("provider_tokens", "moatless-identity"),
    ("repositories", "moatless-workspaces"),
    ("sandbox_unreported_commands", "moatless-sandbox-control"),
    ("sandboxes", "moatless-sandbox-control"),
    ("schedules", "moatless-loops"),
    ("secret_audit_events", "moatless-identity"),
    ("secrets", "moatless-identity"),
    ("subscriptions", "moatless-loops"),
    ("tags", "moatless-tasks"),
    ("task_activities", "moatless-tasks"),
    ("task_bindings", "moatless-loops"),
    ("task_conversation_changes", "moatless-core"),
    ("task_conversation_position", "moatless-core"),
    ("task_event_changes", "moatless-core"),
    ("task_event_dispatch_queue", "moatless-core"),
    ("task_event_position", "moatless-core"),
    ("task_events", "moatless-tasks"),
    ("task_follows", "moatless-tasks"),
    ("task_forks", "moatless-tasks"),
    ("task_inbox", "moatless-tasks"),
    ("task_listing_changes", "moatless-core"),
    ("task_listing_position", "moatless-core"),
    ("task_message_content_blocks", "moatless-tasks"),
    ("task_message_contexts", "moatless-tasks"),
    ("task_message_usage", "moatless-tasks"),
    ("task_messages", "moatless-tasks"),
    ("task_outbound_events", "moatless-tasks"),
    ("task_read_receipts", "moatless-tasks"),
    ("task_relations", "moatless-tasks"),
    ("task_tags", "moatless-tasks"),
    ("task_turns", "moatless-tasks"),
    ("tasks", "moatless-tasks"),
    ("team_members", "moatless-identity"),
    ("teams", "moatless-identity"),
    ("terminal_session_changes", "moatless-core"),
    ("terminal_session_position", "moatless-core"),
    ("trajectory_events", "moatless-tasks"),
    ("trajectory_sessions", "moatless-tasks"),
    ("user_credentials", "moatless-identity"),
    ("user_provider_ids", "moatless-identity"),
    ("users", "moatless-identity"),
    ("workspace_credentials", "moatless-workspaces"),
    ("workspace_repos", "moatless-workspaces"),
    ("workspaces", "moatless-workspaces"),
];

fn allowed(consumer: &str, table: &str, write: bool) -> bool {
    matches!(
        (consumer, table),
        ("moatless-tasks", "sandboxes") | ("moatless-sandbox-control", "tasks")
    ) || (consumer == "moatless-t3code" && !write)
}

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let mut out = Vec::new();
    let mut live = BTreeSet::new();
    if let Some(core) = ws.member("moatless-core") {
        let dir = core.dir.join("migrations");
        if dir.is_dir() {
            let mut paths = std::fs::read_dir(&dir)?
                .map(|p| p.map(|p| p.path()))
                .collect::<std::io::Result<Vec<_>>>()?;
            paths.sort();
            for p in paths
                .into_iter()
                .filter(|p| p.extension().is_some_and(|e| e == "sql"))
            {
                sql::migrate(&mut live, &std::fs::read_to_string(p)?);
            }
            let mapped: BTreeSet<String> = TABLES.iter().map(|(t, _)| (*t).to_string()).collect();
            for table in live.symmetric_difference(&mapped) {
                out.push(Violation::at(
                    RULE,
                    format!("ownership map: {table}"),
                    "migration history and table ownership disagree",
                    ws.relative(&dir),
                ));
            }
        }
    }
    for r in &ws.index()?.records {
        if r.test
            || matches!(
                r.package.as_str(),
                "moatless-backend" | "moatless-cli" | "moatless-sandbox"
            )
            || annotations::permits(r, RULE)
        {
            continue;
        }
        for literal in sql::literals(&r.item) {
            for (table, write) in sql::tables(&literal) {
                if let Some((_, owner)) = TABLES.iter().find(|(t, _)| *t == table) {
                    if *owner != r.package && !allowed(&r.package, &table, write) {
                        out.push(Violation::at(
                            RULE,
                            format!("{} -> {table} ({owner})", r.id),
                            format!("{} names {table}, owned by {owner}", r.package),
                            &r.file,
                        ));
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projections_may_read_but_never_write_foreign_tables() {
        assert!(allowed("moatless-t3code", "tasks", false));
        assert!(!allowed("moatless-t3code", "tasks", true));
        assert!(allowed("moatless-tasks", "sandboxes", true));
        assert!(!allowed("moatless-identity", "sandboxes", false));
    }
}
