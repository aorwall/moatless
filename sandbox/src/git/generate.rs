//! One-shot generation of a commit subject/body and a pull-request title/body
//! from the staged change, for a stacked action the caller gave no message.
//!
//! A single `claude_agent_sdk::query` in its own subprocess with a JSON output
//! schema. The result never touches `total_cost_usd` and nothing gates it: this
//! is work the person pressing the button is not waiting on a turn for, so it
//! runs beside whatever the thread's Agent is doing and is not counted against
//! it.

use claude_agent_sdk::types::messages::ResultMessage;
use claude_agent_sdk::types::options::{OutputFormat, SystemPrompt};
use claude_agent_sdk::{ClaudeOptions, SdkMessage, query};
use serde_json::json;

/// The messages a stacked action needs when the caller supplied none.
#[derive(Debug, Clone)]
pub struct GeneratedMessages {
    pub commit_subject: String,
    pub commit_body: Option<String>,
    pub pr_title: String,
    pub pr_body: String,
}

/// What the staged change looks like, as context for generation. Both the
/// summary and the patch are capped by the caller before they get here.
pub struct GenerationContext {
    pub staged_summary: String,
    pub staged_patch: String,
    /// The repository's recent commit subjects, newest first — the only signal
    /// that can match a `commitlint` config stricter than any preset.
    pub recent_subjects: Vec<String>,
    /// The repository's pull-request template, when it has one.
    pub pr_template: Option<String>,
}

const SYSTEM_PROMPT: &str = "You write a git commit message and a pull-request \
title and body for a staged change. Follow the style of the repository's recent \
commit subjects exactly — if they are Conventional Commits, match that grammar \
and scope vocabulary. Keep the commit subject under 72 characters and in the \
imperative mood. Do not invent changes the diff does not show. Reply only with \
the requested JSON.";

fn output_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "commitSubject": { "type": "string" },
            "commitBody": { "type": "string" },
            "prTitle": { "type": "string" },
            "prBody": { "type": "string" }
        },
        "required": ["commitSubject", "prTitle", "prBody"],
        "additionalProperties": false
    })
}

fn build_prompt(ctx: &GenerationContext) -> String {
    let mut prompt = String::new();
    if !ctx.recent_subjects.is_empty() {
        prompt.push_str("Recent commit subjects in this repository (newest first):\n");
        for subject in &ctx.recent_subjects {
            prompt.push_str("- ");
            prompt.push_str(subject);
            prompt.push('\n');
        }
        prompt.push('\n');
    }
    if let Some(template) = &ctx.pr_template {
        prompt.push_str("The repository's pull-request template:\n");
        prompt.push_str(template);
        prompt.push_str("\n\n");
    }
    prompt.push_str("Staged change summary:\n");
    prompt.push_str(&ctx.staged_summary);
    prompt.push_str("\n\nStaged patch:\n");
    prompt.push_str(&ctx.staged_patch);
    prompt
}

/// Generate the messages. `Err` carries a human-readable reason the commit
/// phase reports; the caller decides what to do with it.
pub async fn generate_messages(
    repo_dir: &str,
    cli_path: &str,
    model: Option<&str>,
    ctx: &GenerationContext,
) -> Result<GeneratedMessages, String> {
    let options = ClaudeOptions {
        model: model.map(str::to_string),
        system_prompt: Some(SystemPrompt::Replace(SYSTEM_PROMPT.to_string())),
        cwd: Some(repo_dir.to_string()),
        cli_path: Some(cli_path.to_string()),
        // Structured output costs a turn of its own: the model calls the
        // `StructuredOutput` tool and needs a further turn to end the run, so a
        // ceiling of one fails with "Reached maximum number of turns".
        max_turns: Some(2),
        output_format: Some(OutputFormat::JsonSchema {
            schema: output_schema(),
        }),
        ..ClaudeOptions::default()
    };

    let mut handle = query(&build_prompt(ctx), options)
        .await
        .map_err(|e| format!("could not start message generation: {e}"))?;

    while let Some(message) = handle.messages.recv().await {
        match message {
            Ok(SdkMessage::Result(ResultMessage::Success(result))) => {
                let value = result
                    .structured_output
                    .or_else(|| serde_json::from_str(&result.result).ok())
                    .ok_or_else(|| "message generation returned no JSON".to_string())?;
                return parse_generated(&value);
            }
            Ok(SdkMessage::Result(ResultMessage::Error(error))) => {
                let detail = error.errors.join("; ");
                let detail = if detail.is_empty() {
                    error.subtype
                } else {
                    detail
                };
                return Err(format!("message generation failed: {detail}"));
            }
            Ok(_) => {}
            Err(e) => return Err(format!("message generation stream error: {e}")),
        }
    }
    Err("message generation ended without a result".to_string())
}

fn parse_generated(value: &serde_json::Value) -> Result<GeneratedMessages, String> {
    let field = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let commit_subject = field("commitSubject")
        .ok_or_else(|| "generated message had no commit subject".to_string())?;
    let pr_title = field("prTitle").unwrap_or_else(|| commit_subject.clone());
    let pr_body = field("prBody").unwrap_or_default();
    Ok(GeneratedMessages {
        commit_subject,
        commit_body: field("commitBody"),
        pr_title,
        pr_body,
    })
}
