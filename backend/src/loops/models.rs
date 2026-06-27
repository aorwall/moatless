//! Loop domain types — the parsed shape of a `LOOP.md` file.

use serde::Deserialize;

/// YAML frontmatter of a `LOOP.md`. Field names mirror the authoring surface
/// documented for `.moatless/loops/<name>/LOOP.md` (camelCase keys).
///
/// `on` is the only required field; everything else has a default applied
/// during sync. Unknown keys are ignored so the format can grow without
/// breaking older backends.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopFrontmatter {
    /// Event source: `github_pr`, `schedule`, `slack`, or `linear`.
    pub on: String,
    /// Optional override for `triggers.name` (defaults to the loop dir name).
    #[serde(default)]
    pub name: Option<String>,
    /// Agent type (`trigger_rules.agent_type`, default `claude-code`).
    #[serde(default)]
    pub agent_type: Option<String>,
    /// Routing mode (`trigger_rules.routing_mode`, default `by_subject`).
    #[serde(default)]
    pub routing_mode: Option<String>,
    /// Tag names resolved to ids for `trigger_rules.tag_ids`.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Explicit run-as user id; see resolution order in `sync.rs`.
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
    /// (e.g. `["Ready"]`). Maps to `provider_config.trigger_states`.
    #[serde(default)]
    pub trigger_states: Option<Vec<String>>,
    /// `on: linear` only — Linear project UUIDs to restrict task creation to.
    /// Maps to `provider_config.project_ids`. Empty/absent = any project.
    #[serde(default)]
    pub project_ids: Option<Vec<String>>,
    /// `on: linear` only — issue must carry every label here to spawn a task.
    /// Maps to `provider_config.require_labels`.
    #[serde(default)]
    pub require_labels: Option<Vec<String>>,
    /// `on: linear` only — workflow-state NAME to move the issue into when a
    /// task is created (e.g. `"In Progress"`). Maps to
    /// `provider_config.start_state`.
    #[serde(default)]
    pub start_state: Option<String>,
    /// `on: linear` only — whether an `Issue.update` that moves the issue into a
    /// `trigger_states` value spawns a task. Maps to
    /// `provider_config.create_on_status_entry`. `None` keeps the default-on
    /// behavior (and the `adapterConfig` escape hatch); set `false` to author a
    /// mention-mode loop where only `@mention`/assignment spawns tasks.
    #[serde(default)]
    pub create_on_status_entry: Option<bool>,
    /// Adapter-event filter JSON (default `[]`).
    #[serde(default)]
    pub event_filter: Option<serde_json::Value>,
    /// Adapter-specific config JSON stored on the trigger source.
    #[serde(default)]
    pub adapter_config: Option<serde_json::Value>,
    /// Whether the trigger is active (default `true`).
    #[serde(default)]
    pub active: Option<bool>,
    /// Per-trigger task inactivity TTL in days (`trigger_rules.inactive_ttl_days`).
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
#[derive(Debug, Clone)]
pub struct ParsedLoop {
    pub name: String,
    pub frontmatter: LoopFrontmatter,
    pub prompt_body: String,
}
