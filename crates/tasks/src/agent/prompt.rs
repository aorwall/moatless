//! Append-system-prompt assembly for agent runs.
//!
//! The backend builds the entire `append` system prompt sent to the agent.
//! The sandbox fetches it separately and appends the backend-supplied string
//! verbatim.

use std::sync::Arc;

use tracing::warn;

use moatless_backend_core::workspace::WorkspaceResolver;
use moatless_core::config::Config;
use moatless_types::workspace::ServerConfig;

use crate::task::models::Task;
use crate::task::relations::TaskRelationDao;
use moatless_types::task::TaskRelationType;

/// The Task this one hangs off, and how — the two shapes of a parent edge.
///
/// They read the same in a listing and differently here: a spawned child owes
/// its parent a result, and a fork owes nobody anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentTask<'a> {
    /// Spawned by that Task, which is waiting to be told how it went.
    CreatedBy(&'a str),
    /// Branched off that Task at one of its turns, carrying its conversation
    /// up to that point. `at_turn` is `None` when the cut was at head.
    ForkedFrom {
        task_id: &'a str,
        at_turn: Option<i32>,
    },
}

/// Inputs needed to build the append-system-prompt for one task.
pub struct PromptInputs<'a> {
    pub task_id: &'a str,
    /// `Task.name` — `None` and whitespace-only treated as missing.
    pub task_name: Option<&'a str>,
    /// Set when this task hangs off another one.
    pub parent: Option<ParentTask<'a>>,
    /// `Task.append_system_prompt` — trigger-set instructions.
    pub task_append_system_prompt: Option<&'a str>,
    /// The workspace's `system_prompt`.
    pub repo_system_prompt: Option<&'a str>,
    pub servers: &'a [ServerConfig],
    pub frontend_url: Option<&'a str>,
    pub proxy_domain: Option<&'a str>,
    pub proxy_protocol: &'a str,
}

/// Steers the agent toward `moat gh` (the backend-proxied GitHub wrapper) and
/// away from the (broken) hosted GitHub MCP tools. `moat gh`
/// proxies the mutations that need backend side effects, records what a raw
/// `gh api` POST creates, and passes everything else through to the
/// authenticated `gh` CLI.
const GITHUB_CLI_HINT: &str = "\
# GitHub Access

All GitHub access goes through `moat gh`, run via bash — do not rely on GitHub
MCP tools. For these four operations you MUST use `moat gh` (the backend
proxies them and binds the PR/issue to this task, so review comments and
replies route back here automatically):

- `moat gh pr create --title \"<title>\" --body \"<body>\"`
- `moat gh pr comment <number> --body \"<text>\"`
- `moat gh pr reply <pr_number> <comment_id> --body \"<text>\"`
- `moat gh issue comment <number> --body \"<text>\"`

`-R owner/repo` and `--head` are inferred from the current git repository when
omitted. Every other gh subcommand (`pr view`, `api`, `run watch`, …) passes
through to the local authenticated `gh` CLI unchanged, so `moat gh <anything>`
is always safe to use — and a `moat gh api` POST that creates a review comment,
a review or an issue comment is recorded for this task on your behalf.";

/// CLI reference appended at the end of the system prompt.
const SANDBOX_TOOLS_PROMPT: &str = "\
# Inbound Message Headers

User messages may carry one of two header shapes so you can tell where they
came from and respond appropriately.

## External (message originated from an adapter — Slack, Telegram, Linear, GitHub PR)

```
<message_id>msg_abc123</message_id>
<source>slack</source>

…the actual body…
```

Reply on the original channel with:

```
moat messages reply --message msg_abc123 --text \"…\"
```

The backend routes the reply to the right surface based on `<source>`:
- `slack` → reply in the same Slack thread
- `telegram` → reply on the same Telegram message
- `linear` → comment on the same Linear issue
- `github_pr` → threaded review-comment reply (or top-level PR comment for
  conversation comments)

You can also `moat messages react --message msg_abc123 --emoji thumbsup` to
acknowledge with a reaction instead of a reply.

## Internal (UI / CLI / scheduler — no external channel)

```
<sent_by>alice</sent_by>

…the actual body…
```

`<sent_by>` is the moatless login of the user who initiated the message.
There is no external channel to call back to — just emit a normal assistant
response and the user sees it directly in the task UI. Do NOT call
`moat messages reply` for internal messages.

When neither header is present (e.g. system/scheduler-originated rows with
no resolvable sender), treat the body as authoritative on its own.

# Sandbox CLI Reference

Available `moat` commands:
- `moat set-name \"<name>\"` — Update the task name
- `moat set-tags <tag1> <tag2> ...` — Set tags on this task
- `moat messages reply --message <id> --text \"<text>\"` — Reply to a specific
  external message (carries `<message_id>` + `<source>`). Backend picks the
  destination automatically.
- `moat messages react --message <id> --emoji <name>` — React to a specific
  external message with an emoji. Pass `--remove` to undo.
- `moat messages list $TASK_ID --full` — Read this task's conversation history
  as a full-text transcript (newest 50 by default; `--limit <n>` up to 200,
  `--before-id <id>` for older pages). Use it to recover context when prior
  conversation is missing from your session.
- `moat gh pr create --title \"<t>\" --body \"<b>\"` — Create a GitHub PR via the
  backend (binds the PR to this task; `-R`/`--head` inferred from the cwd repo)
- `moat gh pr comment <n> --body \"<text>\"` — Comment on a PR via the backend
- `moat gh pr reply <n> <comment_id> --body \"<text>\"` — Threaded reply to a PR
  review comment via the backend (moat-only; ids appear in delivered messages)
- `moat gh issue comment <n> --body \"<text>\"` — Comment on an issue via the backend
- `moat gh <any other gh args>` — Passthrough to the authenticated `gh` CLI
- `moat tasks status $TASK_ID` — Show sandbox status including server states
- `moat tasks logs $TASK_ID --server <name>` — Stream server logs (e.g., `--server preview`)
- `moat tasks diagnose $TASK_ID` — Diagnose sandbox issues (status + container diagnostics + recent errors)
- `moat feedback --comment \"<text>\" [--message <uuid>] [--tool-use-id <id>]` — Record feedback about this task for later review. Use this when the user expresses frustration, when you got stuck in a long troubleshooting loop that better skills or tooling would have prevented, or when an instruction or tool clearly led you wrong. Be specific: name the skill, instruction, or tool, and explain what would have helped.";

/// Build the append-system-prompt for a task.
///
/// Loads the task's workspace (for `system_prompt` + `servers`) and reads URL
/// settings off `Config`, then delegates to [`build_append_system_prompt`].
/// Falls back to a minimal prompt (no workspace or server data) if the
/// workspace lookup fails or workspaces aren't wired.
pub async fn build_for_task(
    task_id: &str,
    task: Option<&Task>,
    workspaces: Option<&Arc<dyn WorkspaceResolver>>,
    task_relations: Option<&Arc<dyn TaskRelationDao>>,
    config: Option<&Arc<Config>>,
) -> String {
    let workspace = match (task, workspaces) {
        (Some(t), Some(ws)) => match t.workspace_id.as_deref() {
            Some(ws_id) => ws.resolve(ws_id).await.ok().flatten(),
            None => None,
        },
        _ => None,
    };

    // Held as an owned pair because `ParentTask` borrows the id, and the
    // relation row does not outlive this scope.
    let parent_relation: Option<(TaskRelationType, String, Option<i32>)> = match task_relations {
        Some(repo) => match repo.find_parent(task_id).await {
            Ok(Some(r)) => {
                let at_turn = r
                    .context
                    .get("at_turn")
                    .and_then(serde_json::Value::as_i64)
                    .and_then(|n| i32::try_from(n).ok());
                Some((r.relation_type, r.to_task_id, at_turn))
            }
            Ok(None) => None,
            Err(e) => {
                warn!(error = %e, task_id, "Failed to load parent relation for prompt");
                None
            }
        },
        None => None,
    };
    let parent = parent_relation
        .as_ref()
        .and_then(|(kind, id, at_turn)| match kind {
            TaskRelationType::CreatedBy => Some(ParentTask::CreatedBy(id)),
            TaskRelationType::ForkedFrom => Some(ParentTask::ForkedFrom {
                task_id: id,
                at_turn: *at_turn,
            }),
            // A peer link is not a parent and says nothing to the agent.
            TaskRelationType::RelatedTo => None,
        });

    let servers: Vec<ServerConfig> = workspace
        .as_ref()
        .and_then(|w| w.servers.clone())
        .unwrap_or_default();

    let frontend_url = config.map(|c| c.auth.frontend_url.as_str());
    let proxy_domain = config.and_then(|c| c.proxy.domain.as_deref());
    let proxy_protocol = config.map_or("https", |c| c.proxy.protocol.as_str());

    build_append_system_prompt(&PromptInputs {
        task_id,
        task_name: task.and_then(|t| t.name.as_deref()),
        parent,
        task_append_system_prompt: task.and_then(|t| t.append_system_prompt.as_deref()),
        repo_system_prompt: workspace.as_ref().and_then(|w| w.system_prompt.as_deref()),
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

    if let Some(parent) = inputs.parent {
        sections.push(build_parent_task_section(parent));
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

        // A server whose port the workspace never declared has none allocated
        // yet, so it has no URL to give the agent.
        if let Some(port) = server.port {
            lines.push(format!("## {label} (port {port})"));
            lines.push(format!(
                "- **Local URL**: http://localhost:{port} (use this to curl/verify the server — no auth needed)"
            ));
            if let Some(domain) = proxy_domain {
                let url = format!("{proxy_protocol}://{task_id}--{port}.{domain}");
                lines.push(format!(
                    "- **External URL**: {url} (share with the user to open in their browser — they are already authenticated)"
                ));
            }
        } else {
            lines.push(format!("## {label} (port assigned at startup)"));
        }

        if !server.start_commands.is_empty() {
            let joined = server.start_commands.join(" && ");
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

/// Build the "# Parent Task" or "# Forked Task" section.
fn build_parent_task_section(parent: ParentTask<'_>) -> String {
    match parent {
        ParentTask::CreatedBy(parent_task_id) => format!(
            "# Parent Task\n\n\
             This task was created by another task (`{parent_task_id}`). \
             When you have completed your work, you MUST report back to the parent task by running:\n\n  \
             `moat send --task {parent_task_id} --text \"<summary of what you did and the outcome>\"`\n\n\
             Always send this final message; the parent task is waiting for your response. \
             It is an AI agent, not a person: send it once, when the work is done, and keep it \
             to the outcome, what it has to act on, and where the work is. Send no progress \
             updates along the way."
        ),
        ParentTask::ForkedFrom { task_id, at_turn } => {
            let cut = match at_turn {
                Some(turn) => format!("at turn {turn}"),
                None => "at its most recent turn".to_string(),
            };
            format!(
                "# Forked Task\n\n\
                 This task was forked from `{task_id}` {cut}. The conversation above it is a copy \
                 of that task's, up to the fork point — you are continuing from there along a \
                 different path, and anything `{task_id}` did after the fork is not yours.\n\n\
                 Nothing is waiting on your result, so do not report back."
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use moatless_types::workspace::Port;

    use super::*;

    fn base_inputs(task_id: &str) -> PromptInputs<'_> {
        PromptInputs {
            task_id,
            task_name: None,
            parent: None,
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
            port: Port::new(3000),
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
    fn repo_and_trigger_prompts_both_included() {
        let inputs = PromptInputs {
            repo_system_prompt: Some("REPO PROMPT TEXT"),
            task_append_system_prompt: Some("TRIGGER PROMPT TEXT"),
            ..base_inputs("task-123")
        };
        let prompt = build_append_system_prompt(&inputs);
        assert!(prompt.contains("REPO PROMPT TEXT"));
        assert!(prompt.contains("TRIGGER PROMPT TEXT"));
        let repo_idx = prompt.find("REPO PROMPT TEXT").unwrap();
        let trigger_idx = prompt.find("TRIGGER PROMPT TEXT").unwrap();
        assert!(
            repo_idx < trigger_idx,
            "repo prompt must come before trigger prompt"
        );
    }

    #[test]
    fn always_includes_github_and_sandbox_tools_reference() {
        let inputs = base_inputs("task-123");
        let prompt = build_append_system_prompt(&inputs);
        assert!(prompt.contains("# GitHub Access"));
        assert!(prompt.contains("# Sandbox CLI Reference"));
        assert!(prompt.contains("moat messages reply --message"));
        assert!(prompt.contains("moat messages react --message"));
        // External-message header explanation
        assert!(prompt.contains("<message_id>"));
        assert!(prompt.contains("<source>"));
        // Internal-message header explanation
        assert!(prompt.contains("<sent_by>"));
        // The unified guidance lives in the base prompt, not tacked on
        // per-adapter.
        assert!(prompt.contains("Internal"));
        assert!(prompt.contains("External"));
    }

    #[test]
    fn github_access_mandates_moat_gh_proxy() {
        let inputs = base_inputs("task-123");
        let prompt = build_append_system_prompt(&inputs);
        assert!(prompt.contains("`moat gh pr create"));
        assert!(prompt.contains("moat gh pr comment <number> --body"));
        assert!(prompt.contains("moat gh pr reply <pr_number> <comment_id> --body"));
        assert!(prompt.contains("moat gh issue comment <number> --body"));
        // Passthrough escape hatch is documented so agents never get stuck.
        assert!(prompt.contains("passes\nthrough"));
    }

    #[test]
    fn sandbox_tools_reference_includes_moat_feedback() {
        let inputs = base_inputs("task-123");
        let prompt = build_append_system_prompt(&inputs);
        assert!(prompt.contains("`moat feedback"));
        assert!(prompt.contains("--comment"));
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
            parent: Some(ParentTask::CreatedBy("parent-abc")),
            ..base_inputs("task-123")
        };
        let prompt = build_append_system_prompt(&inputs);
        assert!(prompt.contains("# Parent Task"));
        assert!(prompt.contains("parent-abc"));
        assert!(prompt.contains("moat send --task parent-abc"));
    }

    #[test]
    fn forked_task_section_names_the_source_and_does_not_report_back() {
        let inputs = PromptInputs {
            parent: Some(ParentTask::ForkedFrom {
                task_id: "source-abc",
                at_turn: Some(4),
            }),
            ..base_inputs("task-123")
        };
        let prompt = build_append_system_prompt(&inputs);
        assert!(prompt.contains("# Forked Task"));
        assert!(prompt.contains("source-abc"));
        assert!(prompt.contains("turn 4"));
        assert!(!prompt.contains("# Parent Task"));
        assert!(!prompt.contains("moat send --task source-abc"));
    }

    /// The fork section restating the naming rule is what stopped forks from
    /// getting named: it read "once you know what you are doing here", which
    /// licenses deferring the `MUST` that `# Required First Actions` states
    /// above it, and pointed at a `set-name` that cannot set a description.
    #[test]
    fn the_fork_section_leaves_naming_to_the_required_first_actions() {
        let section = build_parent_task_section(ParentTask::ForkedFrom {
            task_id: "source-abc",
            at_turn: Some(4),
        });
        assert!(!section.contains("set-name"));
    }

    #[test]
    fn an_unnamed_fork_is_told_to_name_itself_before_it_replies() {
        let inputs = PromptInputs {
            parent: Some(ParentTask::ForkedFrom {
                task_id: "source-abc",
                at_turn: Some(4),
            }),
            ..base_inputs("task-123")
        };
        let prompt = build_append_system_prompt(&inputs);
        assert!(prompt.contains("# Required First Actions"));
        assert!(prompt.contains("moat set-name"));
    }
}
