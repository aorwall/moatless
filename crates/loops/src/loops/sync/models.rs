//! Loop domain types — the parsed shape of a `LOOP.md` file.

use serde::Deserialize;

/// YAML frontmatter of a `LOOP.md` (camelCase keys); field names mirror the
/// authoring surface documented for `.moatless/loops/<name>/LOOP.md`.
///
/// `on` and `name` are required; everything else defaults during sync, and
/// unknown keys are ignored so the format can grow without breaking older
/// backends.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopFrontmatter {
    /// Event source: `github_pr`, `schedule`, `slack`, or `linear`.
    pub on: String,
    /// Human-readable name (`loops.name`). Required — the directory name is the
    /// reconciliation key, not a name anybody chose.
    pub name: String,
    /// Name of the Workspace the spawned tasks run in, resolved among the
    /// Workspaces this repository is placed in. See
    /// [`moatless_types::loops::LoopContext::workspace_id`].
    #[serde(default)]
    pub workspace: Option<String>,
    /// Agent type (`loop_configs.agent_type`, default `claude-code`).
    #[serde(default)]
    pub agent_type: Option<String>,
    /// Model the spawned tasks run on (`loop_configs.model`). `None` keeps the
    /// platform default the sandbox resolves for `agent_type`.
    #[serde(default)]
    pub model: Option<String>,
    /// Routing mode (`loop_configs.routing_mode`, default `by_subject`).
    #[serde(default)]
    pub routing_mode: Option<String>,
    /// Tag names resolved to ids for `loop_configs.tag_ids`.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Skill names every task this Loop spawns activates on each fresh agent
    /// session (`loop_configs.skills`). Stored verbatim — a skill resolves on
    /// the pod that runs the task, so there is nothing to look up here.
    #[serde(default)]
    pub skills: Option<Vec<String>>,
    /// Legacy Git field retained for compatibility. Sync reports and ignores it
    /// because only an administrator may approve the run-as identity.
    #[serde(default)]
    pub run_as_user_id: Option<String>,
    /// Override the adapter-event `source_matcher` (default `owner/repo`).
    #[serde(default)]
    pub source_matcher: Option<String>,
    /// Slack workspace/team id for resolving the adapter connection (`on:
    /// slack`). For `on: linear`, the Linear **workspace/organization id**,
    /// used to resolve the connection when more than one is installed; omit
    /// when there's exactly one.
    #[serde(default)]
    pub team_id: Option<String>,
    /// `on: linear` only — Linear workflow state NAMES whose ENTRY spawns a
    /// task (e.g. `["Ready"]`). Maps to `adapter_config.trigger_states`.
    #[serde(default)]
    pub trigger_states: Option<Vec<String>>,
    /// `on: linear` only — Linear project UUIDs to restrict task creation to.
    /// Maps to `adapter_config.project_ids`. Empty/absent = any project.
    #[serde(default)]
    pub project_ids: Option<Vec<String>>,
    /// `on: linear` only — issue must carry every label here to spawn a task.
    /// Maps to `adapter_config.require_labels`.
    #[serde(default)]
    pub require_labels: Option<Vec<String>>,
    /// `on: linear` only — workflow-state NAME to move the issue into when a
    /// task is created (e.g. `"In Progress"`). Maps to
    /// `adapter_config.start_state`.
    #[serde(default)]
    pub start_state: Option<String>,
    /// `on: linear` only — whether an `Issue.update` into a `trigger_states`
    /// value spawns a task (`adapter_config.create_on_status_entry`). `None`
    /// keeps default-on; `false` makes a mention-mode loop where only
    /// `@mention`/assignment spawns tasks.
    #[serde(default)]
    pub create_on_status_entry: Option<bool>,
    /// Adapter-event filter JSON (default `[]`).
    #[serde(default)]
    pub event_filter: Option<serde_json::Value>,
    /// Adapter-specific config JSON stored on the subscription.
    #[serde(default)]
    pub adapter_config: Option<serde_json::Value>,
    /// Legacy Git field retained for compatibility. Sync reports and ignores it
    /// because Git configuration cannot approve Loop execution.
    #[serde(default)]
    pub active: Option<bool>,
    /// Per-loop task inactivity TTL in days (`loop_configs.inactive_ttl_days`).
    /// `None` inherits the platform default (`TASK_INACTIVE_TIMEOUT_DAYS`).
    #[serde(default)]
    pub inactive_ttl_days: Option<i32>,
    /// Schedule settings — only meaningful when `on: schedule`.
    #[serde(default)]
    pub schedule: Option<LoopSchedule>,
}

/// Schedule block of a `schedule` loop's frontmatter.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopSchedule {
    /// 7-field cron expression (sec min hour dom mon dow year).
    #[serde(default)]
    pub cron: Option<String>,
    /// IANA timezone (default `UTC`).
    #[serde(default)]
    pub timezone: Option<String>,
    /// Kickoff message template (default: "Run the `<loop-name>` loop.").
    #[serde(default)]
    pub message: Option<String>,
}

/// A parsed `LOOP.md`: the directory name (stable upsert key), its parsed
/// frontmatter, and the markdown body (used as the agent's
/// `append_system_prompt`).
///
/// The directory name identifies the Loop across syncs; the displayed name,
/// `frontmatter.name`, may change freely.
#[derive(Debug, Clone)]
pub struct ParsedLoop {
    pub dir_name: String,
    pub frontmatter: LoopFrontmatter,
    pub prompt_body: String,
}
