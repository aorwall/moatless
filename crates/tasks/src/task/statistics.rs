//! Conversation statistics assembled from persisted task messages.

use std::collections::{BTreeMap, HashMap};

use moatless_api_schemas::task::{
    TaskMessageTokenUsage, TaskSkillUsage, TaskStatisticsResponse, TaskTokenTotals,
    TaskToolCallStats,
};

use super::models::Task;
use crate::message::models::{MessageType, TaskMessage};
use crate::task_activities::dao::Activity;
use crate::task_activities::mapping::ActivityKind;

#[derive(Debug, Default)]
struct ToolAccumulator {
    total_calls: i64,
    failed_calls: i64,
}

/// Deduplicated per-message usage plus the cache split needed for the totals.
struct MessageUsage {
    entry: TaskMessageTokenUsage,
    cache_creation_input_tokens: i64,
    cache_read_input_tokens: i64,
}

/// Sum the per-turn totals recorded from each run's terminal result message.
///
/// Returns `None` when no turn carries usage, so callers fall back to the
/// per-message sum (the pre-column behaviour).
fn totals_from_turn_metrics(turn_metrics: &[serde_json::Value]) -> Option<TaskTokenTotals> {
    let field = |m: &serde_json::Value, k: &str| -> i64 {
        m.get("usage")
            .and_then(|u| u.get(k))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0)
    };

    let mut totals = TaskTokenTotals::default();
    let mut saw_any = false;
    for m in turn_metrics {
        if m.get("usage").is_none_or(serde_json::Value::is_null) {
            continue;
        }
        saw_any = true;
        totals.input_tokens += field(m, "input_tokens");
        totals.output_tokens += field(m, "output_tokens");
        totals.cache_creation_input_tokens += field(m, "cache_creation_input_tokens");
        totals.cache_read_input_tokens += field(m, "cache_read_input_tokens");
    }
    if !saw_any {
        return None;
    }
    totals.cached_input_tokens = totals.cache_read_input_tokens;
    totals.total_tokens = totals.input_tokens
        + totals.output_tokens
        + totals.cache_creation_input_tokens
        + totals.cache_read_input_tokens;
    Some(totals)
}

/// Build all stats from the same rows already used by the conversation view.
///
/// `turn_metrics` carries each run's terminal totals and takes precedence over
/// the per-message sum: on the subprocess/NDJSON stream `output_tokens` is
/// sampled before the response finishes, so summing message rows understates
/// output by an order of magnitude. Turn totals also cover sub-agent work,
/// which the message sum deliberately excludes.
///
/// The per-message series is returned either way for the chart, where relative
/// shape matters more than absolute accuracy.
pub fn build_task_statistics(
    task: &Task,
    messages: &[TaskMessage],
    activities: &[Activity],
    turn_metrics: &[serde_json::Value],
) -> TaskStatisticsResponse {
    let mut token_totals = TaskTokenTotals::default();
    // Deduplicated per-message usage, keyed by the Anthropic message id and kept
    // in first-seen order. Two data-quality issues make a naive per-row sum wrong:
    //   1. The agent stream persists the same assistant message several times
    //      (streaming snapshots / resume replays), so summing rows double-counts.
    //   2. Sub-agent (sidechain) messages — those spawned by a Task/Agent tool,
    //      identified by a `parent_uuid` — report *cumulative* session usage on
    //      every row, so a single row can read tens of millions of tokens.
    // We therefore count only main-conversation assistant messages
    // (`parent_uuid IS NULL`) and keep one entry per message id (the largest
    // snapshot, i.e. the final/complete one).
    let mut usage_by_id: HashMap<String, MessageUsage> = HashMap::new();
    let mut usage_order: Vec<String> = Vec::new();
    let mut latest_model: Option<String> = None;
    // Ordered so the two lists come out stable for a client that renders them
    // as-is; the counts themselves do not depend on the order.
    let mut tool_stats: BTreeMap<String, ToolAccumulator> = BTreeMap::new();
    let mut skill_stats: BTreeMap<String, i64> = BTreeMap::new();

    for message in messages {
        let model = message
            .assistant_message_model
            .clone()
            .or_else(|| message.model.clone());
        if model.is_some() {
            latest_model = model.clone();
        }

        if message.message_type == MessageType::Assistant && message.parent_uuid.is_none() {
            let input_tokens = i64::from(message.assistant_usage_input_tokens.unwrap_or(0));
            let output_tokens = i64::from(message.assistant_usage_output_tokens.unwrap_or(0));
            let cache_creation_input_tokens = i64::from(
                message
                    .assistant_usage_cache_creation_input_tokens
                    .unwrap_or(0),
            );
            let cache_read_input_tokens =
                i64::from(message.assistant_usage_cache_read_input_tokens.unwrap_or(0));
            let cached_input_tokens = cache_creation_input_tokens + cache_read_input_tokens;
            let total_tokens = input_tokens + output_tokens + cached_input_tokens;

            if total_tokens > 0 {
                // Fall back to the row uuid when the message id is absent so rows
                // without an Anthropic id are still counted (once).
                let key = message
                    .assistant_message_id
                    .clone()
                    .unwrap_or_else(|| message.uuid.clone());
                let usage = MessageUsage {
                    entry: TaskMessageTokenUsage {
                        message_uuid: message.uuid.clone(),
                        timestamp: message.timestamp.clone(),
                        turn_number: message.turn_number,
                        model: model.clone(),
                        input_tokens,
                        output_tokens,
                        cached_input_tokens,
                        total_tokens,
                    },
                    cache_creation_input_tokens,
                    cache_read_input_tokens,
                };
                match usage_by_id.get(&key) {
                    Some(existing) if existing.entry.total_tokens >= total_tokens => {}
                    Some(_) => {
                        usage_by_id.insert(key, usage);
                    }
                    None => {
                        usage_order.push(key.clone());
                        usage_by_id.insert(key, usage);
                    }
                }
            }
        }
    }

    // One row per call, so the count is the row count and the failure count is a
    // column — no scan of anything, and no second table to cross against.
    for activity in activities {
        if activity.kind != ActivityKind::ToolCall {
            continue;
        }
        let entry = tool_stats.entry(activity.summary.clone()).or_default();
        entry.total_calls += 1;
        if activity.is_error {
            entry.failed_calls += 1;
        }

        if activity.summary == "Skill" {
            if let Some(skill_name) = activity
                .tool_input()
                .get("skill")
                .and_then(serde_json::Value::as_str)
            {
                *skill_stats.entry(skill_name.to_string()).or_insert(0) += 1;
            }
        }
    }

    // Materialize the deduplicated usage in first-seen (chronological) order and
    // derive the totals from the same set so the summary tiles and the per-message
    // chart always agree.
    let mut message_token_usage = Vec::with_capacity(usage_order.len());
    for key in usage_order {
        if let Some(usage) = usage_by_id.remove(&key) {
            token_totals.input_tokens += usage.entry.input_tokens;
            token_totals.output_tokens += usage.entry.output_tokens;
            token_totals.cache_creation_input_tokens += usage.cache_creation_input_tokens;
            token_totals.cache_read_input_tokens += usage.cache_read_input_tokens;
            token_totals.cached_input_tokens += usage.entry.cached_input_tokens;
            token_totals.total_tokens += usage.entry.total_tokens;
            message_token_usage.push(usage.entry);
        }
    }

    TaskStatisticsResponse {
        task_id: task.task_id.clone(),
        agent_type: task.agent_type.clone(),
        model: latest_model,
        token_totals: totals_from_turn_metrics(turn_metrics).unwrap_or(token_totals),
        message_token_usage,
        tool_calls: tool_stats
            .into_iter()
            .map(|(tool_name, stats)| TaskToolCallStats {
                tool_name,
                total_calls: stats.total_calls,
                failed_calls: stats.failed_calls,
            })
            .collect(),
        skills: skill_stats
            .into_iter()
            .map(|(skill_name, total_calls)| TaskSkillUsage {
                skill_name,
                total_calls,
            })
            .collect(),
        // No activities at all means this Task ran before they were stored, and
        // an empty `tool_calls` from it is "not recorded" rather than "made
        // none". Decision 9 backfills nothing, so this is the only way a client
        // can tell those two apart — and reporting zero would be a wrong answer,
        // not a missing one.
        tool_calls_available: !activities.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use serde_json::Value;

    use crate::message::models::{MessageRole, MessageType};
    use crate::task::models::TaskStatus;
    use crate::task_activities::mapping::ActivityStatus;
    use moatless_types::agent::AgentType;
    use moatless_types::task::Visibility;

    fn task() -> Task {
        Task {
            task_id: "task-1".to_string(),
            short_id: None,
            created_at: chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00.000Z")
                .expect("fixture timestamp")
                .with_timezone(&chrono::Utc),
            last_message_at: chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00.000Z")
                .expect("fixture timestamp")
                .with_timezone(&chrono::Utc),
            agent_type: AgentType("codex".to_string()),
            repository_id: "repo-1".to_string(),
            status: TaskStatus::Open,
            total_cost_usd: 0.0,
            site_url: None,
            template_id: None,
            vibe: None,
            name: None,
            description: None,
            site_id: None,
            site_provider: None,
            branch: None,
            model: None,
            created_by: None,
            visibility: Visibility::Private,
            scope: moatless_types::access_control::Scope::User,
            owner_user_id: Some("user-1".to_string()),
            run_as_user_id: None,
            updated_at: None,
            append_system_prompt: None,
            headless: false,
            ttl_minutes: 60,
            inactive_ttl_days: 7,
            loop_id: None,
            workspace_id: None,
            sandbox_id: None,
            checkout_mode: None,
            skills: Vec::new(),
        }
    }

    fn assistant_message() -> TaskMessage {
        TaskMessage {
            id: 11,
            task_id: "task-1".to_string(),
            uuid: "msg-1".to_string(),
            message_type: MessageType::Assistant,
            user_type: "external".to_string(),
            parent_uuid: None,
            is_sidechain: None,
            cwd: None,
            session_id: None,
            turn_number: Some(2),
            version: None,
            git_branch: None,
            request_id: None,
            timestamp: "2026-01-01T00:00:00.000Z".to_string(),
            message_role: MessageRole::Assistant,
            message_text: Some("Working on it.".to_string()),
            assistant_message_id: None,
            assistant_message_type: None,
            assistant_message_model: Some("gpt-5".to_string()),
            assistant_stop_reason: None,
            assistant_stop_sequence: None,
            assistant_usage_input_tokens: Some(10),
            assistant_usage_output_tokens: Some(5),
            assistant_usage_cache_creation_input_tokens: Some(3),
            assistant_usage_cache_read_input_tokens: Some(2),
            assistant_usage: None,
            tool_use_result: None,
            tool_response: None,
            sdk_metadata: None,
            attachments: None,
            agent_session_id: None,
            will_not_resume: None,
            run_metrics: None,
            agent_mode_raw: None,
            effort: None,
            model: None,
            skills: Vec::new(),
            resume: true,
            sent_at: None,
            created_at: None,
            external_subject_key: None,
            external_author: None,
            external_sender_id: None,
            source_task_id: None,
            start_fresh: false,
            sent_by: None,
            external_event_id: None,
            external_adapter: None,
            external_source_name: None,
        }
    }

    fn tool_call(activity_id: &str, position: i64, name: &str, input: Value) -> Activity {
        Activity {
            activity_id: activity_id.to_string(),
            task_id: "task-1".to_string(),
            position,
            ordinal: 0,
            message_uuid: Some("msg-1".to_string()),
            parent_uuid: None,
            kind: ActivityKind::ToolCall,
            status: ActivityStatus::Completed,
            tool_use_id: Some(format!("tu-{position}")),
            summary: name.to_string(),
            payload: serde_json::json!({ "input": input }),
            detail: None,
            is_error: false,
            created_at: DateTime::from_timestamp(1_000, 0).expect("valid ts"),
        }
    }

    #[test]
    fn builds_token_tool_and_skill_stats() {
        let mut bash = tool_call("msg-1:0", 1, "Bash", serde_json::json!({}));
        bash.status = ActivityStatus::Failed;
        bash.is_error = true;
        let skill = tool_call(
            "msg-1:1",
            2,
            "Skill",
            serde_json::json!({ "skill": "backend-dev-guidelines" }),
        );

        let stats = build_task_statistics(&task(), &[assistant_message()], &[bash, skill], &[]);

        assert_eq!(stats.model.as_deref(), Some("gpt-5"));
        assert_eq!(stats.token_totals.input_tokens, 10);
        assert_eq!(stats.token_totals.output_tokens, 5);
        assert_eq!(stats.token_totals.cached_input_tokens, 5);
        assert_eq!(stats.token_totals.total_tokens, 20);
        assert_eq!(stats.message_token_usage.len(), 1);
        assert_eq!(stats.tool_calls.len(), 2);
        assert_eq!(stats.tool_calls[0].tool_name, "Bash");
        assert_eq!(stats.tool_calls[0].failed_calls, 1);
        assert_eq!(stats.skills[0].skill_name, "backend-dev-guidelines");
        assert_eq!(stats.skills[0].total_calls, 1);
        assert!(stats.tool_calls_available);
    }

    /// Decision 9 backfills nothing, so a Task that ran before activities were
    /// stored has none — and must say so rather than report no tool calls.
    #[test]
    fn a_task_with_no_activities_reports_tool_calls_unavailable() {
        let stats = build_task_statistics(&task(), &[assistant_message()], &[], &[]);

        assert!(stats.tool_calls.is_empty());
        assert!(!stats.tool_calls_available);
    }

    #[test]
    fn dedupes_by_message_id_and_excludes_sidechain_usage() {
        // A real main-conversation assistant message.
        let mut a1 = assistant_message();
        a1.uuid = "a1".to_string();
        a1.assistant_message_id = Some("msg_1".to_string());
        a1.assistant_usage_input_tokens = Some(10);
        a1.assistant_usage_output_tokens = Some(5);
        a1.assistant_usage_cache_creation_input_tokens = Some(0);
        a1.assistant_usage_cache_read_input_tokens = Some(100);

        // The same message persisted a second time (streaming/replay duplicate).
        let mut a1_dup = a1.clone();
        a1_dup.uuid = "a1-dup".to_string();

        // A second distinct main-conversation assistant message.
        let mut a2 = a1.clone();
        a2.uuid = "a2".to_string();
        a2.assistant_message_id = Some("msg_2".to_string());
        a2.assistant_usage_output_tokens = Some(7);

        // A sub-agent (sidechain) message — identified by parent_uuid — carrying
        // absurd cumulative usage. Must be excluded entirely.
        let mut sidechain = a1.clone();
        sidechain.uuid = "sc".to_string();
        sidechain.assistant_message_id = Some("msg_sc".to_string());
        sidechain.parent_uuid = Some("toolu_x".to_string());
        sidechain.assistant_usage_output_tokens = Some(343_471);
        sidechain.assistant_usage_cache_read_input_tokens = Some(62_000_000);

        let stats = build_task_statistics(&task(), &[a1, a1_dup, a2, sidechain], &[], &[]);

        // Only the two distinct main-agent messages, counted once each.
        assert_eq!(stats.message_token_usage.len(), 2);
        assert_eq!(stats.token_totals.input_tokens, 20); // 10 + 10, duplicate not re-counted
        assert_eq!(stats.token_totals.output_tokens, 12); // 5 + 7, sidechain excluded
        assert_eq!(stats.token_totals.cache_read_input_tokens, 200); // 100 + 100
        assert_eq!(stats.token_totals.total_tokens, 20 + 12 + 200);
    }

    #[test]
    fn turn_metrics_take_precedence_over_per_message_sum() {
        let a1 = assistant_message();

        // What the terminal result reported for the run. Claude Code's per-message
        // `output_tokens` (5 on the row above) is sampled mid-response; the run
        // actually produced 130.
        let turn = serde_json::json!({
            "usage": {
                "input_tokens": 10,
                "output_tokens": 130,
                "cache_creation_input_tokens": 40,
                "cache_read_input_tokens": 100,
            },
            "modelUsage": { "claude-sonnet-4-6": { "costUSD": 0.01 } }
        });

        let stats = build_task_statistics(&task(), &[a1], &[], &[turn]);

        assert_eq!(stats.token_totals.output_tokens, 130);
        assert_eq!(stats.token_totals.total_tokens, 10 + 130 + 40 + 100);
        // The per-message series is untouched — it still drives the chart.
        assert_eq!(stats.message_token_usage.len(), 1);
        assert_eq!(stats.message_token_usage[0].output_tokens, 5);
    }

    #[test]
    fn turn_metrics_without_usage_fall_back_to_message_sum() {
        let a1 = assistant_message();
        // A turn row that closed without usage (e.g. tmux, which reports none).
        let turn = serde_json::json!({ "durationMs": 1200 });

        let stats = build_task_statistics(&task(), &[a1], &[], &[turn]);

        assert_eq!(stats.token_totals.output_tokens, 5);
    }

    /// The tmux transport reports no per-model breakdown, but its usage totals
    /// are still the run's own. An empty `modelUsage` must not demote them.
    #[test]
    fn turn_metrics_without_a_model_breakdown_still_win() {
        let turn = serde_json::json!({
            "usage": {
                "input_tokens": 36,
                "output_tokens": 239,
                "cache_creation_input_tokens": 44235,
                "cache_read_input_tokens": 0,
            },
            "modelUsage": {},
            "totalCostUsd": 0.0,
        });

        let stats = build_task_statistics(&task(), &[assistant_message()], &[], &[turn]);

        assert_eq!(stats.token_totals.output_tokens, 239);
    }
}
