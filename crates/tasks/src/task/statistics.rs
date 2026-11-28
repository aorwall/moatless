//! Conversation statistics assembled from the conversation read model.

use std::collections::BTreeMap;

use moatless_api_schemas::task::{
    TaskMessageTokenUsage, TaskSkillUsage, TaskStatisticsResponse, TaskTokenTotals,
    TaskToolCallStats,
};

use super::models::Task;
use crate::activities::dao::Activity;
use crate::activities::mapping::ActivityKind;
use crate::conversation::read_dao::ResponseUsage;
use crate::message::models::TaskMessage;

#[derive(Debug, Default)]
struct ToolAccumulator {
    total_calls: i64,
    failed_calls: i64,
}

/// Sum the per-turn totals recorded from each run's terminal result message.
/// Returns `None` when no turn carries usage, so callers fall back to the
/// per-response sum.
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

/// Build all stats from the rows the conversation view already uses.
/// `turn_metrics` takes precedence over the per-response sum, which understates
/// sampled `output_tokens` and excludes sub-agent work.
pub(crate) fn build_task_statistics(
    task: &Task,
    messages: &[TaskMessage],
    usage: &[ResponseUsage],
    activities: &[Activity],
    turn_metrics: &[serde_json::Value],
) -> TaskStatisticsResponse {
    let mut token_totals = TaskTokenTotals::default();
    // Ordered so the two lists come out stable for a client that renders them
    // as-is; the counts themselves do not depend on the order.
    let mut tool_stats: BTreeMap<String, ToolAccumulator> = BTreeMap::new();
    let mut skill_stats: BTreeMap<String, i64> = BTreeMap::new();

    let latest_model = messages.iter().rev().find_map(|message| {
        message
            .assistant_message_model
            .clone()
            .or_else(|| message.model.clone())
    });

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

        if activity.summary == "Skill"
            && let Some(skill_name) = activity
                .tool_input()
                .get("skill")
                .and_then(serde_json::Value::as_str)
        {
            *skill_stats.entry(skill_name.to_string()).or_insert(0) += 1;
        }
    }

    // The summary tiles count every response; the chart only those a Message
    // names.
    let mut message_token_usage = Vec::with_capacity(usage.len());
    for response in usage {
        let cached_input_tokens =
            response.cache_creation_input_tokens + response.cache_read_input_tokens;
        let total_tokens = response.input_tokens + response.output_tokens + cached_input_tokens;
        if total_tokens == 0 {
            continue;
        }
        token_totals.input_tokens += response.input_tokens;
        token_totals.output_tokens += response.output_tokens;
        token_totals.cache_creation_input_tokens += response.cache_creation_input_tokens;
        token_totals.cache_read_input_tokens += response.cache_read_input_tokens;
        token_totals.cached_input_tokens += cached_input_tokens;
        token_totals.total_tokens += total_tokens;
        let Some(message_uuid) = response.message_uuid else {
            continue;
        };
        message_token_usage.push(TaskMessageTokenUsage {
            message_uuid: message_uuid.to_string(),
            timestamp: response
                .created_at
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            turn_number: response.turn_number,
            model: response.model.clone(),
            input_tokens: response.input_tokens,
            output_tokens: response.output_tokens,
            cached_input_tokens,
            total_tokens,
        });
    }

    TaskStatisticsResponse {
        task_id: task.task_id.clone(),
        agent_profile: task.agent_profile,
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
        // No activities means this Task predates activity storage, which was
        // never backfilled: an empty `tool_calls` reads as "not recorded," not
        // "made none". Reporting zero would be a wrong answer, not a missing
        // one.
        tool_calls_available: !activities.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use serde_json::Value;

    use crate::activities::mapping::ActivityStatus;
    use crate::message::models::{MessageRole, MessageType};
    use crate::task::models::TaskStatus;
    use moatless_types::agent::AgentProfile;
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
            agent_profile: AgentProfile::Codex,
            repository_id: Some("repo-1".to_string()),
            status: TaskStatus::Open,
            total_cost_usd: 0.0,
            template_id: None,
            name: None,
            description: None,
            branch: None,
            model: None,
            created_by: None,
            reporter_user_id: None,
            visibility: Visibility::Private,
            owner_user_id: Some("user-1".to_string()),
            run_as_user_id: None,
            updated_at: None,
            append_system_prompt: None,
            headless: false,
            ttl_minutes: 60,
            close_after_settled_days: 7,
            settle_after_hours: None,
            loop_id: None,
            workspace_id: None,
            sandbox_id: None,
            checkout_mode: None,
            memory_preload: Vec::new(),
            agent_properties: Default::default(),
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
            sandbox_wake: false,
            parent_uuid: None,
            session_id: None,
            turn_number: Some(2),
            timestamp: "2026-01-01T00:00:00.000Z".to_string(),
            message_role: MessageRole::Assistant,
            message_text: Some("Working on it.".to_string()),
            assistant_message_id: None,
            assistant_message_model: Some("gpt-5".to_string()),
            assistant_stop_reason: None,
            assistant_stop_sequence: None,
            assistant_usage_input_tokens: None,
            assistant_usage_output_tokens: None,
            assistant_usage_cache_creation_input_tokens: None,
            assistant_usage_cache_read_input_tokens: None,
            tool_use_result: None,
            tool_response: None,
            sdk_metadata: None,
            attachments: None,
            agent_session_id: None,
            will_not_resume: None,
            run_metrics: None,
            agent_mode: None,
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

    fn response(input: i64, output: i64, cache_write: i64, cache_read: i64) -> ResponseUsage {
        ResponseUsage {
            message_uuid: Some(uuid::Uuid::nil()),
            created_at: DateTime::from_timestamp(1_000, 0).expect("valid ts"),
            turn_number: Some(2),
            model: Some("gpt-5".to_string()),
            input_tokens: input,
            output_tokens: output,
            cache_creation_input_tokens: cache_write,
            cache_read_input_tokens: cache_read,
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

        let stats = build_task_statistics(
            &task(),
            &[assistant_message()],
            &[response(10, 5, 3, 2)],
            &[bash, skill],
            &[],
        );

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

    /// Activities were never backfilled, so a Task that ran before they were
    /// stored has none — and must say so rather than report no tool calls.
    #[test]
    fn a_task_with_no_activities_reports_tool_calls_unavailable() {
        let stats = build_task_statistics(&task(), &[assistant_message()], &[], &[], &[]);

        assert!(stats.tool_calls.is_empty());
        assert!(!stats.tool_calls_available);
    }

    #[test]
    fn sums_each_response_once_and_skips_empty_ones() {
        let unnamed = ResponseUsage {
            message_uuid: None,
            ..response(10, 7, 0, 100)
        };
        let usage = [response(10, 5, 0, 100), response(0, 0, 0, 0), unnamed];

        let stats = build_task_statistics(&task(), &[], &usage, &[], &[]);

        assert_eq!(stats.message_token_usage.len(), 1, "no Message names one");
        assert_eq!(stats.token_totals.input_tokens, 20);
        assert_eq!(stats.token_totals.output_tokens, 12);
        assert_eq!(stats.token_totals.cache_read_input_tokens, 200);
        assert_eq!(stats.token_totals.total_tokens, 20 + 12 + 200);
    }

    #[test]
    fn turn_metrics_take_precedence_over_per_message_sum() {
        let a1 = assistant_message();

        // What the terminal result reported for the run. Claude Code's per-response
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

        let stats = build_task_statistics(&task(), &[a1], &[response(10, 5, 3, 2)], &[], &[turn]);

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

        let stats = build_task_statistics(&task(), &[a1], &[response(10, 5, 3, 2)], &[], &[turn]);

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

        let stats = build_task_statistics(&task(), &[assistant_message()], &[], &[], &[turn]);

        assert_eq!(stats.token_totals.output_tokens, 239);
    }
}
