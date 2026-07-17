//! Conversation statistics assembled from persisted task messages.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use moatless_api_schemas::task::{
    TaskMessageTokenUsage, TaskSkillUsage, TaskStatisticsResponse, TaskTokenTotals,
    TaskToolCallStats,
};

use super::models::Task;
use crate::message::models::{ContentBlockType, TaskMessage, TaskMessageContentBlock};

#[derive(Debug, Clone)]
struct ToolCall {
    tool_use_id: String,
    tool_name: String,
    skill_name: Option<String>,
}

#[derive(Debug, Default)]
struct ToolAccumulator {
    total_calls: i64,
    failed_calls: i64,
}

/// Build all stats from the same rows already used by the conversation view.
pub fn build_task_statistics(
    task: &Task,
    messages: &[TaskMessage],
    blocks: &[TaskMessageContentBlock],
) -> TaskStatisticsResponse {
    let failed_tool_ids: BTreeSet<String> = blocks
        .iter()
        .filter(|block| {
            block.block_type == ContentBlockType::ToolResult
                && block.tool_result_is_error.unwrap_or(false)
        })
        .filter_map(|block| block.tool_use_id.clone())
        .collect();

    let blocks_by_message_id = blocks.iter().fold(
        HashMap::<i32, Vec<&TaskMessageContentBlock>>::new(),
        |mut by_message, block| {
            by_message
                .entry(block.task_message_id)
                .or_default()
                .push(block);
            by_message
        },
    );

    let mut token_totals = TaskTokenTotals::default();
    let mut message_token_usage = Vec::new();
    let mut tool_stats: BTreeMap<String, ToolAccumulator> = BTreeMap::new();
    let mut skill_stats: BTreeMap<String, i64> = BTreeMap::new();
    let mut latest_model: Option<String> = None;

    for message in messages {
        let model = message
            .assistant_message_model
            .clone()
            .or_else(|| message.model.clone());
        if model.is_some() {
            latest_model = model.clone();
        }

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
            token_totals.input_tokens += input_tokens;
            token_totals.output_tokens += output_tokens;
            token_totals.cache_creation_input_tokens += cache_creation_input_tokens;
            token_totals.cache_read_input_tokens += cache_read_input_tokens;
            token_totals.cached_input_tokens += cached_input_tokens;
            token_totals.total_tokens += total_tokens;

            message_token_usage.push(TaskMessageTokenUsage {
                message_uuid: message.uuid.clone(),
                timestamp: message.timestamp.clone(),
                turn_number: message.turn_number,
                model: model.clone(),
                input_tokens,
                output_tokens,
                cached_input_tokens,
                total_tokens,
            });
        }

        for call in extract_tool_calls(
            message,
            blocks_by_message_id
                .get(&message.id)
                .map(std::vec::Vec::as_slice),
        ) {
            let entry = tool_stats.entry(call.tool_name.clone()).or_default();
            entry.total_calls += 1;
            if failed_tool_ids.contains(&call.tool_use_id) {
                entry.failed_calls += 1;
            }

            if let Some(skill_name) = call.skill_name {
                *skill_stats.entry(skill_name).or_insert(0) += 1;
            }
        }
    }

    TaskStatisticsResponse {
        task_id: task.task_id.clone(),
        agent_type: task.agent_type.clone(),
        model: latest_model,
        token_totals,
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
    }
}

fn extract_tool_calls(
    message: &TaskMessage,
    blocks: Option<&[&TaskMessageContentBlock]>,
) -> Vec<ToolCall> {
    let mut calls = tool_calls_from_json(&message.message_content);
    if !calls.is_empty() {
        return calls;
    }

    if let Some(blocks) = blocks {
        for block in blocks {
            if block.block_type == ContentBlockType::ToolUse {
                if let Some(tool_use_id) = block.tool_use_id.clone() {
                    calls.push(ToolCall {
                        tool_use_id,
                        tool_name: "unknown".to_string(),
                        skill_name: None,
                    });
                }
            }
        }
    }

    calls
}

fn tool_calls_from_json(value: &serde_json::Value) -> Vec<ToolCall> {
    let Some(blocks) = content_blocks(value) else {
        return Vec::new();
    };

    blocks
        .iter()
        .filter_map(|block| {
            if block.get("type").and_then(serde_json::Value::as_str) != Some("tool_use") {
                return None;
            }

            let tool_use_id = block
                .get("id")
                .or_else(|| block.get("tool_use_id"))
                .or_else(|| block.get("toolUseId"))
                .and_then(serde_json::Value::as_str)?
                .to_string();
            let tool_name = block
                .get("name")
                .or_else(|| block.get("tool_name"))
                .or_else(|| block.get("toolName"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let skill_name = if tool_name == "Skill" {
                block
                    .get("input")
                    .and_then(|input| input.get("skill"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            } else {
                None
            };

            Some(ToolCall {
                tool_use_id,
                tool_name,
                skill_name,
            })
        })
        .collect()
}

fn content_blocks(value: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    match value {
        serde_json::Value::Array(blocks) => Some(blocks),
        serde_json::Value::Object(_) => value
            .get("content")
            .or_else(|| value.get("message_content"))
            .and_then(serde_json::Value::as_array),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::models::{MessageRole, MessageType};
    use crate::shared_models::AgentType;
    use crate::task::models::{SandboxDesiredState, TaskStatus, Visibility};

    fn task() -> Task {
        Task {
            task_id: "task-1".to_string(),
            short_id: None,
            created_at: "2026-01-01T00:00:00.000Z".to_string(),
            last_message_at: "2026-01-01T00:00:00.000Z".to_string(),
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
            created_by: None,
            visibility: Visibility::Private,
            scope: moatless_backend_core::scope::Scope::User,
            team_id: Some("team-1".to_string()),
            owner_user_id: Some("user-1".to_string()),
            created_at_db: None,
            updated_at: None,
            append_system_prompt: None,
            sandbox_started_at: None,
            sandbox_desired_state: SandboxDesiredState::Running,
            sandbox_stop_reason: None,
            sandbox_desired_state_updated_at: chrono::Utc::now(),
            headless: false,
            ttl_minutes: 60,
            inactive_ttl_days: 7,
            task_template_id: None,
            trigger_id: None,
            disabled_servers: None,
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
            message_content: serde_json::json!([
                { "type": "tool_use", "id": "tu-1", "name": "Bash", "input": {} },
                { "type": "tool_use", "id": "tu-2", "name": "Skill", "input": { "skill": "backend-dev-guidelines" } }
            ]),
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
            sdk_metadata: None,
            attachments: None,
            agent_session_id: None,
            will_not_resume: None,
            run_metrics: None,
            agent_mode: None,
            model: None,
            resume: true,
            sent_at: None,
            created_at: None,
            external_event_id: None,
            external_adapter: None,
            external_subject_key: None,
            external_source_name: None,
            command_name: None,
            source_task_id: None,
            start_fresh: false,
            sent_by: None,
        }
    }

    #[test]
    fn builds_token_tool_and_skill_stats() {
        let blocks = vec![TaskMessageContentBlock {
            id: 1,
            task_message_id: 12,
            sort_order: 0,
            block_type: ContentBlockType::ToolResult,
            show_in_ui: false,
            text_content: None,
            tool_use_id: Some("tu-1".to_string()),
            tool_result_content: Some("failed".to_string()),
            tool_result_is_error: Some(true),
            image_media_type: None,
            image_data: None,
            created_at: None,
        }];

        let stats = build_task_statistics(&task(), &[assistant_message()], &blocks);

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
    }
}
