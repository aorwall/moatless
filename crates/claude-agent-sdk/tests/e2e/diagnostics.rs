//! What the CLI reports rather than what it should report. These print and
//! assert a minimal invariant, so a CLI change shows up as output, not a
//! failure.

use crate::harness::{
    Backend, LONG_TEST_TIMEOUT, TEST_MODEL, TEST_TIMEOUT, auth_env, collect_messages, e2e_options,
    extract_assistant_text, extract_result_text, fresh_test_cwd, init_tracing,
};
use claude_agent_sdk::ClaudeOptions;
use claude_agent_sdk::ContentBlock;
use claude_agent_sdk::SdkMessage;
use claude_agent_sdk::api::messages::ResultMessage;
use claude_agent_sdk::query;
use tokio::time::timeout;

/// Reproduces a production bug in the CLI's final `result` message.
///
/// Does it reflect trailing `tool_use` text, or come back empty? Only asserts a
/// Result arrives — see `-- --nocapture`.
async fn run_diagnose_empty_result_after_trailing_tool_use(backend: Backend) {
    let tmp = fresh_test_cwd("empty-result");

    let options = ClaudeOptions {
        model: Some(TEST_MODEL.to_string()),
        // Generous turns so the model can: turn1 = text+tool_use, then
        // (optionally) turn2 = no further text. The bug appears if the
        // CLI emits result.result="" even when an assistant text was
        // produced earlier in the same turn.
        max_turns: Some(3),
        allow_dangerously_skip_permissions: Some(true),
        allowed_tools: Some(vec!["Bash".to_string()]),
        cwd: Some(tmp.to_string_lossy().to_string()),
        env: auth_env(),
        ..Default::default()
    };

    // Prompt engineered to mimic the production case:
    //   - Give a textual answer (the "what model are you?" reply)
    //   - Then run a benign Bash command (the "moat set-name" call)
    //   - Explicitly tell the model to stop talking after the tool returns
    let prompt = "Step 1: Answer the question \"what is 2+2?\" with a one-sentence \
                  reply (e.g. \"2+2 equals 4.\"). \
                  Step 2: Run the shell command `echo done` using the Bash tool. \
                  Step 3: After the Bash tool returns, output absolutely no \
                  additional text. Do not summarize, confirm, or comment. Just stop.";

    let mut handle = backend
        .start_one_shot(prompt, options)
        .await
        .expect("query failed");
    let messages = collect_messages(&mut handle.messages).await;

    eprintln!("\n=== EMPTY-RESULT DIAGNOSTIC ({backend:?}) ===");
    eprintln!("Total messages: {}", messages.len());
    for (i, msg) in messages.iter().enumerate() {
        match msg {
            SdkMessage::System(init) => {
                eprintln!(
                    "[{i}] System init: model={} session={}",
                    init.model, init.session_id
                );
            }
            SdkMessage::Assistant(a) => {
                eprintln!(
                    "[{i}] Assistant id={:?} model={:?} stop_reason={:?} blocks={}",
                    a.message.id,
                    a.message.model,
                    a.message.stop_reason,
                    a.message.content.len()
                );
                for (b, block) in a.message.content.iter().enumerate() {
                    match block {
                        ContentBlock::Text { text } => {
                            let preview = &text[..text.len().min(120)];
                            eprintln!("      block[{b}] text: {preview:?}");
                        }
                        ContentBlock::ToolUse { id, name, input } => {
                            eprintln!(
                                "      block[{b}] tool_use id={id} name={name} input={}",
                                serde_json::to_string(input).unwrap_or_default()
                            );
                        }
                        other => eprintln!("      block[{b}] {:?}", std::mem::discriminant(other)),
                    }
                }
            }
            SdkMessage::User(u) => {
                eprintln!(
                    "[{i}] User (tool result) blocks={} tool_use_result_present={}",
                    u.message.content.len(),
                    u.tool_use_result.is_some()
                );
                for (b, block) in u.message.content.iter().enumerate() {
                    match block {
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            is_error,
                        } => {
                            let summary = match content {
                                Some(serde_json::Value::String(s)) => s.clone(),
                                Some(other) => other.to_string(),
                                None => "<no content>".to_string(),
                            };
                            let preview = &summary[..summary.len().min(120)];
                            eprintln!(
                                "      block[{b}] tool_result id={tool_use_id} is_error={is_error:?} content={preview:?}"
                            );
                        }
                        ContentBlock::Text { text } => {
                            let preview = &text[..text.len().min(120)];
                            eprintln!("      block[{b}] text: {preview:?}");
                        }
                        other => eprintln!("      block[{b}] {:?}", std::mem::discriminant(other)),
                    }
                }
            }
            SdkMessage::Result(ResultMessage::Success(s)) => {
                eprintln!(
                    "[{i}] Result:Success turns={} duration={}ms cost=${:.4} is_error={}",
                    s.num_turns, s.duration_ms, s.total_cost_usd, s.is_error
                );
                eprintln!("      result.result.len() = {}", s.result.len());
                eprintln!("      result.result = {:?}", s.result);
                eprintln!(
                    "      model_usage keys: {:?}",
                    s.model_usage.keys().collect::<Vec<_>>()
                );
            }
            SdkMessage::Result(ResultMessage::Error(e)) => {
                eprintln!(
                    "[{i}] Result:Error subtype={} errors={:?}",
                    e.subtype, e.errors
                );
            }
            other => eprintln!("[{i}] {:?}", std::mem::discriminant(other)),
        }
    }

    // ── Cross-check: did assistant emit text? did it emit a tool_use? ──
    let assistant_text = extract_assistant_text(&messages);
    let assistant_emitted_tool_use = messages.iter().any(|m| {
        if let SdkMessage::Assistant(a) = m {
            a.message
                .content
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolUse { .. }))
        } else {
            false
        }
    });
    let result_text = extract_result_text(&messages).unwrap_or_default();

    eprintln!("--- summary ---");
    eprintln!(
        "assistant emitted text? {} (len={})",
        !assistant_text.is_empty(),
        assistant_text.len()
    );
    eprintln!("assistant emitted tool_use? {assistant_emitted_tool_use}");
    eprintln!(
        "result.result empty? {} (len={})",
        result_text.is_empty(),
        result_text.len()
    );
    eprintln!("=== END DIAGNOSTIC ===\n");

    assert!(
        messages.iter().any(|m| matches!(m, SdkMessage::Result(_))),
        "Expected a Result message"
    );
}

#[tokio::test]
#[ignore]
async fn test_diagnose_empty_result_after_trailing_tool_use_subprocess() {
    skip_unless_subprocess_e2e!();
    init_tracing();
    timeout(
        LONG_TEST_TIMEOUT,
        run_diagnose_empty_result_after_trailing_tool_use(Backend::Subprocess),
    )
    .await
    .expect("Test timed out");
}

#[tokio::test]
#[ignore]
async fn test_diagnose_empty_result_after_trailing_tool_use_tmux() {
    skip_unless_tmux_e2e!();
    init_tracing();
    timeout(
        LONG_TEST_TIMEOUT,
        run_diagnose_empty_result_after_trailing_tool_use(Backend::Tmux),
    )
    .await
    .expect("Test timed out");
}

/// Diagnostic test to troubleshoot missing cost data. Prints detailed
/// cost/usage information from the CLI result message.
#[tokio::test]
#[ignore]
async fn test_diagnose_cost_fields() {
    skip_unless_subprocess_e2e!();
    init_tracing();

    let result = timeout(TEST_TIMEOUT, async {
        let mut handle = query("Say hi.", e2e_options())
            .await
            .expect("Failed to start query");

        let messages = collect_messages(&mut handle.messages).await;

        let has_result = messages.iter().any(|m| matches!(m, SdkMessage::Result(_)));
        eprintln!("\n=== COST DIAGNOSTIC ===");
        eprintln!("Total messages received: {}", messages.len());
        eprintln!("Has Result message: {has_result}");

        if !has_result {
            eprintln!("WARNING: No Result message received!");
            eprintln!("This likely means ResultSuccess deserialization failed.");
            eprintln!("Check warn-level logs above for 'Failed to deserialize SDK message'.");
            panic!("No Result message received — deserialization likely failed");
        }

        match messages.iter().find(|m| matches!(m, SdkMessage::Result(_))) {
            Some(SdkMessage::Result(ResultMessage::Success(s))) => {
                eprintln!("Result type: Success");
                eprintln!("  total_cost_usd: {}", s.total_cost_usd);
                eprintln!("  num_turns: {}", s.num_turns);
                eprintln!("  duration_ms: {}", s.duration_ms);
                eprintln!("  is_error: {}", s.is_error);
                eprintln!("  usage.input_tokens: {}", s.usage.input_tokens);
                eprintln!("  usage.output_tokens: {}", s.usage.output_tokens);
                eprintln!(
                    "  usage.cache_read_input_tokens: {:?}",
                    s.usage.cache_read_input_tokens
                );
                eprintln!(
                    "  usage.cache_creation_input_tokens: {:?}",
                    s.usage.cache_creation_input_tokens
                );
                eprintln!("  model_usage entries: {}", s.model_usage.len());
                for (model, mu) in &s.model_usage {
                    eprintln!(
                        "    {model}: cost_usd={} in={} out={}",
                        mu.cost_usd, mu.input_tokens, mu.output_tokens
                    );
                }

                let payload = serde_json::to_value(s).unwrap();
                eprintln!(
                    "  payload.total_cost_usd: {:?}",
                    payload.get("total_cost_usd")
                );

                assert!(
                    s.total_cost_usd > 0.0,
                    "Expected non-zero cost, got: {}",
                    s.total_cost_usd
                );
            }
            Some(SdkMessage::Result(ResultMessage::Error(e))) => {
                eprintln!("Result type: Error");
                eprintln!("  subtype: {}", e.subtype);
                eprintln!("  total_cost_usd: {}", e.total_cost_usd);
                eprintln!("  errors: {:?}", e.errors);
                panic!("Got ResultError instead of ResultSuccess");
            }
            _ => unreachable!(),
        }

        eprintln!("=== END DIAGNOSTIC ===\n");
    })
    .await;

    result.expect("Test timed out");
}
