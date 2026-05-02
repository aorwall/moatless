//! Append-system-prompt assembly for agent runs.
//!
//! The backend builds the entire `append` system prompt sent to the agent
//! and stamps it onto every `PendingMessage`. The sandbox is a passive
//! recipient — it appends the backend-supplied string verbatim.

use std::sync::Arc;

use tracing::warn;

use crate::config::Config;
use crate::repository::dao::RepoRepository;
use crate::sandbox::models::ServerConfig;
use crate::task::models::Task;

/// Inputs needed to build the append-system-prompt for one task.
pub struct PromptInputs<'a> {
    pub task_id: &'a str,
    /// `Task.name` — `None` and whitespace-only treated as missing.
    pub task_name: Option<&'a str>,
    /// Set when this task was spawned by another task.
    pub parent_task_id: Option<&'a str>,
    /// `Task.append_system_prompt` — channel-set instructions.
    pub task_append_system_prompt: Option<&'a str>,
    /// `Repository.system_prompt`.
    pub repo_system_prompt: Option<&'a str>,
    pub servers: &'a [ServerConfig],
    pub frontend_url: Option<&'a str>,
    pub proxy_domain: Option<&'a str>,
    pub proxy_protocol: &'a str,
}

/// Steers the agent away from the (broken) hosted GitHub MCP tools and toward
/// the `gh` CLI, which the sandbox already authenticates from `GITHUB_TOKEN`.
const GITHUB_CLI_HINT: &str = "\
# GitHub Access

The `gh` CLI is authenticated and is the only way to access GitHub from this sandbox.
Use it via bash for all GitHub operations — do not rely on GitHub MCP tools.";

/// CLI reference appended at the end of the system prompt.
const SANDBOX_TOOLS_PROMPT: &str = "\
# Sandbox CLI Reference

Available `moat` commands:
- `moat set-name \"<name>\"` — Update the task name
- `moat set-tags <tag1> <tag2> ...` — Set tags on this task
- `moat tasks status $TASK_ID` — Show sandbox status including server states
- `moat tasks logs $TASK_ID --server <name>` — Stream server logs (e.g., `--server preview`)
- `moat tasks diagnose $TASK_ID` — Diagnose sandbox issues (status + container diagnostics + recent errors)";

/// Build the append-system-prompt for a task.
///
/// Loads the repository (for `system_prompt` + `servers`) and reads URL
/// settings off `Config`, then delegates to [`build_append_system_prompt`].
/// Falls back to a minimal prompt (no repo or server data) if the repo
/// lookup fails or repositories aren't wired.
pub async fn build_for_task(
    task_id: &str,
    task: Option<&Task>,
    repositories: Option<&Arc<dyn RepoRepository>>,
    config: Option<&Arc<Config>>,
) -> String {
    let repo = match (task, repositories) {
        (Some(t), Some(repos)) => repos
            .find_repository_by_id(&t.repository_id)
            .await
            .ok()
            .flatten(),
        _ => None,
    };

    let servers: Vec<ServerConfig> = repo
        .as_ref()
        .and_then(|r| r.servers.clone())
        .map(|value| {
            serde_json::from_value::<Vec<ServerConfig>>(value).unwrap_or_else(|e| {
                warn!(
                    error = %e,
                    task_id,
                    "Failed to parse repository.servers as Vec<ServerConfig>; treating as empty"
                );
                Vec::new()
            })
        })
        .unwrap_or_default();

    let frontend_url = config.map(|c| c.auth.frontend_url.as_str());
    let proxy_domain = config.and_then(|c| c.proxy.domain.as_deref());
    let proxy_protocol = config.map_or("https", |c| c.proxy.protocol.as_str());

    build_append_system_prompt(&PromptInputs {
        task_id,
        task_name: task.and_then(|t| t.name.as_deref()),
        parent_task_id: task.and_then(|t| t.parent_task_id.as_deref()),
        task_append_system_prompt: task.and_then(|t| t.append_system_prompt.as_deref()),
        repo_system_prompt: repo.as_ref().and_then(|r| r.system_prompt.as_deref()),
        servers: &servers,
        frontend_url,
        proxy_domain,
        proxy_protocol,
    })
}

/// Build the full append-system-prompt for the given inputs.
pub fn build_append_system_prompt(inputs: &PromptInputs<'_>) -> String {
    let mut sections: Vec<String> = Vec::new();

    sections.push(build_task_identity_section(inputs.task_name));

    if let Some(sp) = inputs.repo_system_prompt {
        sections.push(sp.to_string());
    }

    if let Some(sp) = inputs.task_append_system_prompt {
        sections.push(sp.to_string());
    }

    if let Some(servers_section) = build_server_section(
        inputs.servers,
        inputs.task_id,
        inputs.proxy_domain,
        inputs.proxy_protocol,
    ) {
        sections.push(servers_section);
    }

    if let Some(frontend_url) = inputs.frontend_url {
        sections.push(format!(
            "# Task URL\n\nThis task is available at: {}/tasks/{}",
            frontend_url.trim_end_matches('/'),
            inputs.task_id
        ));
    }

    if let Some(parent_id) = inputs.parent_task_id {
        sections.push(build_parent_task_section(parent_id));
    }

    sections.push(GITHUB_CLI_HINT.to_string());
    sections.push(SANDBOX_TOOLS_PROMPT.to_string());

    sections.join("\n\n")
}

/// Either the "# Task Name" rename hint (when the task already has a name)
/// or the "# Required First Actions" directive (when it doesn't).
fn build_task_identity_section(task_name: Option<&str>) -> String {
    let trimmed = task_name.map(str::trim).filter(|s| !s.is_empty());
    if let Some(name) = trimmed {
        format!(
            "# Task Name\n\nThis task is named **{name}**. If the name no longer reflects what the\n\
             task is about, rename it with:\n  `moat set-name \"<new-name>\"`"
        )
    } else {
        "# Required First Actions\n\n\
         Before responding to the user's first message, you MUST run:\n  \
         `moat set-name \"<name>\"`\n\
         Choose a short, descriptive name based on the user's request (under 50 characters)."
            .to_string()
    }
}

/// Build the "# Preview Servers" section. Returns `None` when `servers` is empty.
fn build_server_section(
    servers: &[ServerConfig],
    task_id: &str,
    proxy_domain: Option<&str>,
    proxy_protocol: &str,
) -> Option<String> {
    if servers.is_empty() {
        return None;
    }

    let mut lines = vec![
        "# Preview Servers".to_string(),
        String::new(),
        "The following preview servers are running as sidecars in this sandbox environment. \
         They start automatically when the workspace is ready — you do not need to start them manually."
            .to_string(),
    ];

    for server in servers {
        lines.push(String::new());
        let label = server.label.as_deref().unwrap_or(&server.name);
        lines.push(format!("## {label} (port {port})", port = server.port));

        lines.push(format!(
            "- **Local URL**: http://localhost:{} (use this to curl/verify the server — no auth needed)",
            server.port
        ));

        if let Some(domain) = proxy_domain {
            let url = format!(
                "{proxy_protocol}://{task_id}--{port}.{domain}",
                port = server.port
            );
            lines.push(format!(
                "- **External URL**: {url} (share with the user to open in their browser — they are already authenticated)"
            ));
        }

        if let Some(ref cmds) = server.start_commands {
            let joined = cmds.join(" && ");
            lines.push(format!(
                "- **Configured start command**: `{joined}` (runs automatically, configured in the repository)"
            ));
        }

        if server.auto_reload {
            lines.push(
                "- **Auto-reload**: enabled (file changes are reflected automatically)".to_string(),
            );
        }
    }

    Some(lines.join("\n"))
}

/// Build the "# Parent Task" section for child tasks.
fn build_parent_task_section(parent_task_id: &str) -> String {
    format!(
        "# Parent Task\n\n\
         This task was created by another task (`{parent_task_id}`). \
         When you have completed your work, you MUST report back to the parent task by running:\n\n  \
         `moat messages send {parent_task_id} --text \"<summary of what you did and the outcome>\"`\n\n\
         Always send this final message — the parent task is waiting for your response."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_inputs<'a>(task_id: &'a str) -> PromptInputs<'a> {
        PromptInputs {
            task_id,
            task_name: None,
            parent_task_id: None,
            task_append_system_prompt: None,
            repo_system_prompt: None,
            servers: &[],
            frontend_url: None,
            proxy_domain: None,
            proxy_protocol: "https",
        }
    }

    #[test]
    fn task_name_set_branch_includes_name_and_no_required_first_actions() {
        let inputs = PromptInputs {
            task_name: Some("Refactor login flow"),
            ..base_inputs("task-123")
        };
        let prompt = build_append_system_prompt(&inputs);
        assert!(prompt.contains("# Task Name"));
        assert!(prompt.contains("**Refactor login flow**"));
        assert!(prompt.contains("rename it with"));
        assert!(!prompt.contains("# Required First Actions"));
    }

    #[test]
    fn task_name_missing_branch_includes_required_first_actions() {
        let inputs = base_inputs("task-123");
        let prompt = build_append_system_prompt(&inputs);
        assert!(prompt.contains("# Required First Actions"));
        assert!(prompt.contains("`moat set-name"));
        assert!(!prompt.contains("# Task Name"));
    }

    #[test]
    fn empty_string_name_treated_as_missing() {
        let inputs = PromptInputs {
            task_name: Some(""),
            ..base_inputs("task-123")
        };
        let prompt = build_append_system_prompt(&inputs);
        assert!(prompt.contains("# Required First Actions"));
        assert!(!prompt.contains("# Task Name"));
    }

    #[test]
    fn whitespace_only_name_treated_as_missing() {
        let inputs = PromptInputs {
            task_name: Some("   \t\n "),
            ..base_inputs("task-123")
        };
        let prompt = build_append_system_prompt(&inputs);
        assert!(prompt.contains("# Required First Actions"));
        assert!(!prompt.contains("# Task Name"));
    }

    #[test]
    fn server_section_omitted_when_servers_empty() {
        let inputs = base_inputs("task-123");
        let prompt = build_append_system_prompt(&inputs);
        assert!(!prompt.contains("# Preview Servers"));
    }

    #[test]
    fn server_section_includes_external_url_when_proxy_domain_set() {
        let server = ServerConfig {
            name: "preview".to_string(),
            label: Some("Preview".to_string()),
            port: 3000,
            auto_reload: true,
            ..Default::default()
        };
        let servers = [server];
        let inputs = PromptInputs {
            servers: &servers,
            proxy_domain: Some("moatless.xyz"),
            proxy_protocol: "https",
            ..base_inputs("task-abc")
        };
        let prompt = build_append_system_prompt(&inputs);
        assert!(prompt.contains("# Preview Servers"));
        assert!(prompt.contains("## Preview (port 3000)"));
        assert!(prompt.contains("http://localhost:3000"));
        assert!(prompt.contains("https://task-abc--3000.moatless.xyz"));
        assert!(prompt.contains("Auto-reload"));
    }

    #[test]
    fn task_url_omitted_when_frontend_url_missing() {
        let inputs = base_inputs("task-123");
        let prompt = build_append_system_prompt(&inputs);
        assert!(!prompt.contains("# Task URL"));
    }

    #[test]
    fn task_url_included_when_frontend_url_set_and_trims_trailing_slash() {
        let inputs = PromptInputs {
            frontend_url: Some("https://moatless.example.com/"),
            ..base_inputs("task-xyz")
        };
        let prompt = build_append_system_prompt(&inputs);
        assert!(prompt.contains("# Task URL"));
        assert!(prompt.contains("https://moatless.example.com/tasks/task-xyz"));
    }

    #[test]
    fn repo_and_channel_prompts_both_included() {
        let inputs = PromptInputs {
            repo_system_prompt: Some("REPO PROMPT TEXT"),
            task_append_system_prompt: Some("CHANNEL PROMPT TEXT"),
            ..base_inputs("task-123")
        };
        let prompt = build_append_system_prompt(&inputs);
        assert!(prompt.contains("REPO PROMPT TEXT"));
        assert!(prompt.contains("CHANNEL PROMPT TEXT"));
        let repo_idx = prompt.find("REPO PROMPT TEXT").unwrap();
        let channel_idx = prompt.find("CHANNEL PROMPT TEXT").unwrap();
        assert!(
            repo_idx < channel_idx,
            "repo prompt must come before channel prompt"
        );
    }

    #[test]
    fn always_includes_github_and_sandbox_tools_reference() {
        let inputs = base_inputs("task-123");
        let prompt = build_append_system_prompt(&inputs);
        assert!(prompt.contains("# GitHub Access"));
        assert!(prompt.contains("# Sandbox CLI Reference"));
    }

    #[test]
    fn parent_task_section_omitted_when_no_parent() {
        let inputs = base_inputs("task-123");
        let prompt = build_append_system_prompt(&inputs);
        assert!(!prompt.contains("# Parent Task"));
    }

    #[test]
    fn parent_task_section_included_with_send_command() {
        let inputs = PromptInputs {
            parent_task_id: Some("parent-abc"),
            ..base_inputs("task-123")
        };
        let prompt = build_append_system_prompt(&inputs);
        assert!(prompt.contains("# Parent Task"));
        assert!(prompt.contains("parent-abc"));
        assert!(prompt.contains("moat messages send parent-abc"));
    }
}
