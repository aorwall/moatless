//! Loop domain types — the parsed shape of a `LOOP.md` file.

use serde::Deserialize;

/// YAML frontmatter of a `LOOP.md`. Field names mirror the authoring surface
/// documented for `.moatless/loops/<name>/LOOP.md` (camelCase keys).
///
/// `on` and `name` are the required fields; everything else has a default
/// applied during sync. Unknown keys are ignored so the format can grow without
/// breaking older backends.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopFrontmatter {
    /// Event source: `github_pr`, `schedule`, `slack`, or `linear`.
    pub on: String,
    /// Human-readable name (`loops.name`). Required — the directory name is
    /// the reconciliation key, not a name anybody chose.
    pub name: String,
    /// Agent type (`loop_configs.agent_type`, default `claude-code`).
    #[serde(default)]
    pub agent_type: Option<String>,
    /// Routing mode (`loop_configs.routing_mode`, default `by_subject`).
    #[serde(default)]
    pub routing_mode: Option<String>,
    /// Tag names resolved to ids for `loop_configs.tag_ids`.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Legacy Git field retained for compatibility. Sync reports and ignores
    /// it because only an administrator may approve the run-as identity.
    #[serde(default)]
    pub run_as_user_id: Option<String>,
    /// Override the adapter-event `source_matcher` (default `owner/repo`).
    #[serde(default)]
    pub source_matcher: Option<String>,
    /// Slack workspace/team id for resolving the adapter connection (`on: slack`).
    /// For `on: linear` this is the Linear **workspace/organization id** used to
    /// resolve the Linear connection when more than one is installed; omit it
    /// when there's exactly one Linear connection.
    #[serde(default)]
    pub team_id: Option<String>,
    /// `on: linear` only — Linear workflow state NAMES whose ENTRY spawns a task
    /// (e.g. `["Ready"]`). Maps to `adapter_config.trigger_states`.
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
    /// `on: linear` only — whether an `Issue.update` that moves the issue into a
    /// `trigger_states` value spawns a task. Maps to
    /// `adapter_config.create_on_status_entry`. `None` keeps the default-on
    /// behavior (and the `adapterConfig` escape hatch); set `false` to author a
    /// mention-mode loop where only `@mention`/assignment spawns tasks.
    #[serde(default)]
    pub create_on_status_entry: Option<bool>,
    /// Adapter-event filter JSON (default `[]`).
    #[serde(default)]
    pub event_filter: Option<serde_json::Value>,
    /// Adapter-specific config JSON stored on the subscription.
    #[serde(default)]
    pub adapter_config: Option<serde_json::Value>,
    /// Legacy Git field retained for compatibility. Sync reports and ignores
    /// it because Git configuration cannot approve Loop execution.
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
/// The directory name identifies the Loop across syncs; the name it is
/// displayed under is `frontmatter.name` and may change freely.
#[derive(Debug, Clone)]
pub struct ParsedLoop {
    pub dir_name: String,
    pub frontmatter: LoopFrontmatter,
    pub prompt_body: String,
}
