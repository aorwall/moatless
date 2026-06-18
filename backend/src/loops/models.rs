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
    /// Event source: `github_pr` or `schedule`.
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
