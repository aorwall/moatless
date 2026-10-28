//! Append-system-prompt assembly for agent runs.
//!
//! The backend builds the entire `append` system prompt sent to the agent. The
//! sandbox fetches it separately and appends the backend-supplied string
//! verbatim.

use std::sync::Arc;

use tracing::warn;

use moatless_core::config::Config;
use moatless_ports::identity::UserResolver;
use moatless_workspace_ports::WorkspaceResolver;

use crate::memory::operations::Memories;
use crate::memory::preload::preload_index;
use crate::task::fork::FORK_NAME_PREFIX;
use crate::task::models::Task;
use crate::task::relations::TaskRelations;
use moatless_types::task::TaskRelationType;

/// The Task this one hangs off, and how — the two shapes of a parent edge.
///
/// They read the same in a listing and differently here: a spawned child owes
/// its parent a result, and a fork owes nobody anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentTask<'a> {
    /// Spawned by that Task, which is waiting to be told how it went.
    CreatedBy(&'a str),
    /// Branched off that Task at one of its turns, carrying its conversation up
    /// to that point. `at_turn` is `None` when the cut was at head.
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
    /// Moatless login of the person the task runs for.
    pub task_user: Option<&'a str>,
    /// Set when this task hangs off another one.
    pub parent: Option<ParentTask<'a>>,
    /// `Task.append_system_prompt` — trigger-set instructions.
    pub task_append_system_prompt: Option<&'a str>,
    /// The workspace's `system_prompt`.
    pub repo_system_prompt: Option<&'a str>,
    pub frontend_url: Option<&'a str>,
    /// Whether the backend has a decision provider, so `moat decide` answers.
    pub decisions_enabled: bool,
    /// Whether the backend has object storage, so `moat artifact publish`
    /// answers.
    pub artifacts_enabled: bool,
    /// The summaries [`crate::memory::preload::preload_index`] lists for the
    /// task's `memory_preload`.
    pub memory_index: Option<&'a str>,
}

/// Steers the agent toward `moat gh` and away from the (broken) hosted GitHub
/// MCP tools.
///
/// `moat gh` proxies mutations that need backend side effects, records what a
/// raw `gh api` POST creates, and passes everything else through to the
/// authenticated `gh` CLI.
const GITHUB_CLI_HINT: &str = "\
# GitHub Access

All GitHub access goes through `moat gh`, run via bash, not the hosted GitHub
MCP tools. Use `moat gh` for these four operations: the backend proxies them and
binds the PR/issue to this task, so review comments and replies route back here
automatically:

- `moat gh pr create --title \"<title>\" --body \"<body>\"`
- `moat gh pr comment <number> --body \"<text>\"`
- `moat gh pr reply <pr_number> <comment_id> --body \"<text>\"` (a comment id
  naming a top-level conversation comment, which has no thread, is posted as a
  new PR comment instead)
- `moat gh issue comment <number> --body \"<text>\"`

`-R owner/repo` and `--head` are inferred from the current git repository when
omitted, so run these from the checkout of the PR's own repository or pass
`-R`. Every other gh subcommand (`pr view`, `api`, `run watch`, …) passes
through to the local authenticated `gh` CLI unchanged, so `moat gh <anything>`
is always safe to use — and a `moat gh api` POST that creates a review comment,
a review or an issue comment is recorded for this task on your behalf.

Do not run `gh auth setup-git`: git already has a credential helper for
github.com, and replacing it can leave git asking `gh`, which holds only a
placeholder token, so every later fetch and push fails with `Invalid username
or token`.";

/// CLI reference appended at the end of the system prompt.
const SANDBOX_TOOLS_PROMPT: &str = "\
# Inbound Message Headers

User messages from an external channel carry a header so you can tell where they
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
- `artifact` → a record a reader saved on one of this task's Artifact pages.
  The reply is saved on that Artifact as the record `agent-reply/<message id>`,
  which the page shows, as the `artifacts` skill describes.

You can also `moat messages react --message msg_abc123 --emoji thumbsup` to
acknowledge with a reaction instead of a reply.

On `slack`, attach a file to the reply with `--file <path>` (repeatable). Files
the sender shared arrive on disk; their paths are listed with the message.

## Internal (UI / CLI / scheduler — no external channel)

A message with no header came from the task UI, the CLI or the scheduler, and is
authoritative on its own. There is no external channel to call back to — just
emit a normal assistant response and the user sees it directly in the task UI.
Internal messages need no `moat messages reply`.

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
  review comment via the backend (moat-only; ids appear in delivered messages;
  a conversation-comment id is posted as a new PR comment instead)
- `moat gh issue comment <n> --body \"<text>\"` — Comment on an issue via the backend
- `moat gh <any other gh args>` — Passthrough to the authenticated `gh` CLI
- `moat tasks status $TASK_ID` — Show sandbox status
- `moat tasks status <id> --result` — Where a task's work stands (working, waiting for children, done) and its latest result
- `moat tasks cancel <id>` — Stop a task's run and its descendants', discarding the messages still waiting for them
- `moat tasks logs $TASK_ID` — Stream the sandbox container's logs
- `moat tasks diagnose $TASK_ID` — Diagnose sandbox issues (status + container diagnostics + recent errors)
- `moat feedback --category <category> --comment \"<text>\"` — Record what cost you time that a fix would have prevented (see `# Feedback Inbox`)";

/// Names `moat decide pick` where it pays off; sent only when a decision
/// provider is configured.
const DECISIONS_PROMPT: &str = "\
# Cheap Decisions

When you have to sort many items into a fixed set of labels (routing issues to
repositories, labelling tickets, triaging failures), use `moat decide pick`
rather than judging each one yourself:

```
moat decide pick \"Which repository owns this issue?\" --options api,web,infra --state \"<text>\"
moat decide pick \"Bug or feature?\" --options bug,feature --each < items.jsonl
```

Describe an option with `--option name=\"when it applies\"`. `--each` reads JSONL
or a JSON array of `{\"id\": ..., \"state\": \"...\"}` objects and prints one TSV row
per item. Act on band `act`, double-check `confirm`, and decide `escalate`
yourself.

The question, options, descriptions and text all go to an external model
provider: never send secrets, credentials or personal data. If `moat decide` is not a recognised command in this sandbox,
skip it.";

/// Asks the agent what cost it time this run that a fix would have prevented.
/// Its own section, after the CLI list, so it reads as an instruction with a
/// trigger and a scope.
const FEEDBACK_PROMPT: &str = "\
# Feedback Inbox

If something here cost you time and a fix somewhere would have prevented it,
say so. Run:

```
moat feedback --category <category> --comment \"<text>\"
```

Pick the category that names what is wrong:

- `environment` — the sandbox itself: a dependency or service is missing, or a
  command you needed to verify your work cannot run here.
- `codebase` — the repository's own code or tooling: something in it that could
  be fixed or improved so this kind of work goes better.
- `instruction` — instructions that are missing, wrong, or pointed you the
  wrong way: `AGENTS.md`, `CLAUDE.md`, or a skill — including a skill that
  should exist and does not, or one that never fired.
- `other` — none of the above.

Be concrete: what you hit, and what would have prevented it. Name the skill,
instruction, or tool. Problems with the code you are changing belong in your PR
or your reply to the user, not here. Things you simply had to think about do not
belong here either. Say nothing if nothing comes to mind; an empty inbox is a
fine result.";

/// Names `moat artifact publish` and when to use it; sent only when the
/// backend has object storage to keep Artifacts in.
const ARTIFACTS_PROMPT: &str = "\
# Artifacts

`moat artifact publish <file.html> --title \"<title>\"` publishes an HTML page as
an Artifact: a page stored outside the sandbox, so it stays available after the
sandbox stops, and visible to any signed-in user even when the task is private.
Use it when a page would be clearer than terminal text, or when the person or
their team would use the page rather than only read it, and give the person the
URL it prints. Leave it public; `--private` limits it to those who can see the
task, and is only for a page that shows a secret, a credential, personal data, or
other tasks' content a reader could not otherwise see.
On a public page any signed-in user can read and save its records, and with
`--notify` send you a message.
`--inline` shows the page in this conversation, `--data` lets readers save
input on the page, and `--notify` sends you each save as a message with
`<source>artifact</source>`.

Before you write a page, load the `artifacts` skill: it covers building the
page, the inline layout rules and interactive pages. Without that skill, read
`moat artifact publish --help`.";

/// Names the `moat memory` commands. Entries reach the prompt only through a
/// Loop's `memory.preload`, appended after this text.
const MEMORY_PROMPT: &str = "\
# Memory

`moat memory` keeps what you learn for later Tasks: a decision and why it was
made, a convention, a fact about the system that took effort to establish, or
the state of a long-running job. Store only what has no other owner; code,
commits, PRs, issues and docs already remember their own contents.

- `moat memory list [--prefix <p>]` and `moat memory search \"<words>\"` list
  one-line summaries; `moat memory get <key>` reads an entry in full.
- `moat memory put <key> --summary \"<one line>\" --text \"<body>\" [--cite <url>]`
  writes an entry. Name it `area:topic`, cite where each fact came from, and
  pass `--if-version <n>` when you update something you read.
- `moat memory append <key> --text \"<line>\"` adds a dated line to its log, creating the entry when it does not exist.
- `moat memory verify <key>` records that its citations still hold.
- `moat memory claim <key> --ttl 2h` and `moat memory release <key>` stop two
  Tasks working the same item. A claim refuses only another claim, so claim
  before you start and treat a refused claim as someone else's work.

Every command reaches one scope: this task's Loop, else this task, unless
`--scope workspace|repository|user|global|task|loop` names another. Pass the
same `--scope` to read an entry back. A Loop's `memory.preload` reads only its
`loop` scope. Entries expire after 28 days unless verified, updated or written
with `--no-expiry`. Memory can be wrong or stale: check an entry's citations
before you act on it.";

/// Build the append-system-prompt for a task.
///
/// Loads the task's workspace and URL settings, then delegates to
/// [`build_append_system_prompt`]; falls back to a minimal prompt if the
/// workspace lookup fails or workspaces aren't wired.
pub async fn build_for_task(
    task_id: &str,
    task: Option<&Task>,
    workspaces: Option<&Arc<dyn WorkspaceResolver>>,
    task_relations: Option<&TaskRelations>,
    users: Option<&dyn UserResolver>,
    config: Option<&Arc<Config>>,
    memories: Option<&Memories>,
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

    let task_user = match (
        task.and_then(|t| t.created_by.as_deref().or(t.owner_user_id.as_deref()))
            .and_then(|id| id.parse::<uuid::Uuid>().ok()),
        users,
    ) {
        // Parsed first because `run_as_target` also matches a login, which a
        // legacy provider id could collide with.
        (Some(user_id), Some(users)) => match users.run_as_target(&user_id.to_string()).await {
            Ok(user) => user.map(|u| u.login),
            Err(e) => {
                warn!(error = %e, task_id, "Failed to resolve task user for prompt");
                None
            }
        },
        _ => None,
    };

    let frontend_url = config.map(|c| c.auth.frontend_url.as_str());

    let memory_index = match (task, memories) {
        (Some(t), Some(m)) => preload_index(m, t).await,
        _ => None,
    };

    build_append_system_prompt(&PromptInputs {
        task_id,
        task_name: task.and_then(|t| t.name.as_deref()),
        task_user: task_user.as_deref(),
        parent,
        task_append_system_prompt: task.and_then(|t| t.append_system_prompt.as_deref()),
        repo_system_prompt: workspace.as_ref().and_then(|w| w.system_prompt.as_deref()),
        frontend_url,
        decisions_enabled: config.is_some_and(|c| c.decision.provider.is_some()),
        artifacts_enabled: config.is_some_and(|c| c.s3.bucket_name.is_some()),
        memory_index: memory_index.as_deref(),
    })
}

/// Build the full append-system-prompt for the given inputs.
pub fn build_append_system_prompt(inputs: &PromptInputs<'_>) -> String {
    let mut sections: Vec<String> = Vec::new();

    let forked = matches!(inputs.parent, Some(ParentTask::ForkedFrom { .. }));
    sections.push(build_task_identity_section(inputs.task_name, forked));

    if let Some(login) = inputs.task_user {
        sections.push(format!(
            "# Task User\n\nThis task runs for the Moatless user `{login}`. A message \
             without a header does not identify its sender: anyone with access to the \
             task, or the scheduler, may have sent it."
        ));
    }

    if let Some(sp) = inputs.repo_system_prompt {
        sections.push(sp.to_string());
    }

    if let Some(sp) = inputs.task_append_system_prompt {
        sections.push(sp.to_string());
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
    if inputs.decisions_enabled {
        sections.push(DECISIONS_PROMPT.to_string());
    }
    if inputs.artifacts_enabled {
        sections.push(ARTIFACTS_PROMPT.to_string());
    }
    match inputs.memory_index {
        Some(index) => sections.push(format!("{MEMORY_PROMPT}\n\n{index}")),
        None => sections.push(MEMORY_PROMPT.to_string()),
    }
    sections.push(FEEDBACK_PROMPT.to_string());

    sections.join("\n\n")
}

/// The "# Task Name" rename hint for a task with a name of its own, else the
/// "# Required First Actions" directive. A fork still on the placeholder
/// [`FORK_NAME_PREFIX`] name gets the directive too.
fn build_task_identity_section(task_name: Option<&str>, forked: bool) -> String {
    let trimmed = task_name.map(str::trim).filter(|s| !s.is_empty());
    if let Some(name) = trimmed.filter(|name| forked && name.starts_with(FORK_NAME_PREFIX)) {
        format!(
            "# Required First Actions\n\n\
             This task is named **{name}**, a placeholder it got when it was forked. \
             Before you reply to the first message, run:\n  \
             `moat set-name \"<name>\"`\n\
             Choose a short, descriptive name for what this fork is trying, based on its \
             first message (under 50 characters)."
        )
    } else if let Some(name) = trimmed {
        format!(
            "# Task Name\n\nThis task is named **{name}**. If the name no longer reflects what the\n\
             task is about, rename it with:\n  `moat set-name \"<new-name>\"`"
        )
    } else {
        "# Required First Actions\n\n\
         Before you reply to the first message, run:\n  \
         `moat set-name \"<name>\"`\n\
         Choose a short, descriptive name based on the user's request (under 50 characters)."
            .to_string()
    }
}

/// Build the "# Parent Task" or "# Forked Task" section.
fn build_parent_task_section(parent: ParentTask<'_>) -> String {
    match parent {
        ParentTask::CreatedBy(parent_task_id) => format!(
            "# Parent Task\n\n\
             This task was created by another task (`{parent_task_id}`), which is waiting \
             for your result. The backend tells the parent each time one of your turns \
             completes or errors, but that notice carries no result. When your work is \
             done, send your result once:\n\n  \
             `moat send --task {parent_task_id} --text \"<summary of what you did and the outcome>\"`\n\n\
             The parent is an AI agent, not a person: keep the message to the outcome, what \
             it has to act on, and where the work is. Send no progress updates along the way."
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
    use super::*;

    fn base_inputs(task_id: &str) -> PromptInputs<'_> {
        PromptInputs {
            task_id,
            task_name: None,
            task_user: None,
            parent: None,
            task_append_system_prompt: None,
            repo_system_prompt: None,
            frontend_url: None,
            decisions_enabled: false,
            artifacts_enabled: false,
            memory_index: None,
        }
    }

    #[test]
    fn memory_index_follows_the_memory_commands_only_when_preloaded() {
        let bare = build_append_system_prompt(&base_inputs("t"));
        assert!(bare.contains("# Memory"));
        assert!(!bare.contains("## Preloaded memory"));

        let index = "## Preloaded memory\n\n- ci:flaky (loop, 2026-10-01): retry once";
        let prompt = build_append_system_prompt(&PromptInputs {
            memory_index: Some(index),
            ..base_inputs("t")
        });
        let commands = prompt.find("# Memory").unwrap_or(usize::MAX);
        let preloaded = prompt.find("## Preloaded memory").unwrap_or(0);
        assert!(commands < preloaded, "{prompt}");
    }

    #[test]
    fn artifacts_section_only_when_object_storage_is_configured() {
        let off = build_append_system_prompt(&base_inputs("task-123"));
        assert!(!off.contains("moat artifact"), "{off}");

        let on = build_append_system_prompt(&PromptInputs {
            artifacts_enabled: true,
            ..base_inputs("task-123")
        });
        assert!(on.contains("# Artifacts"), "{on}");
        assert!(on.contains("moat artifact publish <file.html>"), "{on}");
        assert!(on.contains("`artifacts` skill"), "{on}");
    }

    #[test]
    fn decisions_section_only_when_a_provider_is_configured() {
        let off = build_append_system_prompt(&base_inputs("task-123"));
        assert!(!off.contains("moat decide"), "{off}");

        let on = build_append_system_prompt(&PromptInputs {
            decisions_enabled: true,
            ..base_inputs("task-123")
        });
        assert!(on.contains("moat decide pick"), "{on}");
        assert!(on.contains("never send secrets, credentials"), "{on}");
        assert!(
            on.contains("not a recognised command in this sandbox"),
            "{on}"
        );
        assert!(
            on.find("\n# Cheap Decisions") < on.find("\n# Feedback Inbox"),
            "{on}"
        );
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
    fn task_user_named_when_resolved() {
        let inputs = PromptInputs {
            task_user: Some("alice"),
            ..base_inputs("task-123")
        };
        let prompt = build_append_system_prompt(&inputs);
        assert!(prompt.contains("# Task User"));
        assert!(prompt.contains("Moatless user `alice`"));
        assert!(prompt.contains("does not identify its sender"));
        assert!(!prompt.contains("<sent_by>"));
        assert!(!build_append_system_prompt(&base_inputs("task-123")).contains("# Task User"));
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
        assert!(prompt.contains("<message_id>"));
        assert!(prompt.contains("<source>"));
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
        assert!(prompt.contains("Do not run `gh auth setup-git`"));
    }

    #[test]
    fn feedback_inbox_is_its_own_section_after_the_cli_reference() {
        let inputs = base_inputs("task-123");
        let prompt = build_append_system_prompt(&inputs);
        assert!(prompt.contains("# Feedback Inbox"));
        assert!(prompt.contains("moat feedback --category <category> --comment"));
        // Every `FeedbackCategory` variant must be named, or the agent cannot
        // pick one: the CLI rejects an unknown value.
        for category in ["`environment`", "`codebase`", "`instruction`", "`other`"] {
            assert!(prompt.contains(category), "prompt must name {category}");
        }
        assert!(prompt.contains("an empty inbox is a\nfine result"));
        let cli_idx = prompt.find("# Sandbox CLI Reference").unwrap();
        let inbox_idx = prompt.find("# Feedback Inbox").unwrap();
        assert!(
            cli_idx < inbox_idx,
            "inbox section must follow the CLI list"
        );
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

    /// Do not let the fork section restate the naming rule permissively: that
    /// licenses deferring the directive in `# Required First Actions` above it,
    /// and a `set-name` pointer here cannot set a description either.
    #[test]
    fn the_fork_section_leaves_naming_to_the_required_first_actions() {
        let section = build_parent_task_section(ParentTask::ForkedFrom {
            task_id: "source-abc",
            at_turn: Some(4),
        });
        assert!(!section.contains("set-name"));
    }

    #[test]
    fn a_fork_on_its_placeholder_name_is_told_to_rename_before_it_replies() {
        let inputs = PromptInputs {
            task_name: Some("Fork: Fix login"),
            parent: Some(ParentTask::ForkedFrom {
                task_id: "source-abc",
                at_turn: Some(4),
            }),
            ..base_inputs("task-123")
        };
        let prompt = build_append_system_prompt(&inputs);
        assert!(prompt.contains("# Required First Actions"));
        assert!(prompt.contains("**Fork: Fix login**"));
        assert!(!prompt.contains("# Task Name"));
    }

    /// Only a fork's name is a placeholder: a Task a person named `Fork: …`
    /// keeps the ordinary rename hint.
    #[test]
    fn a_fork_prefixed_name_on_a_task_that_is_not_a_fork_is_its_own() {
        let inputs = PromptInputs {
            task_name: Some("Fork: the repo"),
            ..base_inputs("task-123")
        };
        let prompt = build_append_system_prompt(&inputs);
        assert!(prompt.contains("# Task Name"));
        assert!(!prompt.contains("# Required First Actions"));
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
