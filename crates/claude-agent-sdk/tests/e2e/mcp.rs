//! An in-process `SdkMcpServer`: that the CLI discovers its tool and calls it.

use crate::harness::{
    TEST_MODEL, TEST_TIMEOUT, auth_env, collect_messages, extract_assistant_text,
    extract_result_text, init_tracing,
};
use claude_agent_sdk::ClaudeOptions;
use claude_agent_sdk::ContentBlock;
use claude_agent_sdk::SdkMessage;
use claude_agent_sdk::api::messages::ResultMessage;
use claude_agent_sdk::mcp::SdkMcpServer;
use claude_agent_sdk::mcp::SdkMcpTool;
use claude_agent_sdk::mcp::ToolResult;
use claude_agent_sdk::query;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use tokio::time::timeout;

#[tokio::test]
#[ignore]
async fn test_sdk_mcp_tool_is_called() {
    skip_unless_subprocess_e2e!();
    init_tracing();

    let result = timeout(TEST_TIMEOUT, async {
        let tool_was_called = Arc::new(AtomicBool::new(false));
        let tool_flag = tool_was_called.clone();

        let server = SdkMcpServer::builder("test-calc")
            .tool(
                SdkMcpTool::new("add", "Add two numbers together")
                    .input_schema(serde_json::json!({
                        "type": "object",
                        "properties": {
                            "a": {"type": "number", "description": "First number"},
                            "b": {"type": "number", "description": "Second number"}
                        },
                        "required": ["a", "b"]
                    }))
                    .handler(move |args| {
                        let flag = tool_flag.clone();
                        Box::pin(async move {
                            flag.store(true, Ordering::SeqCst);
                            let a = args["a"].as_f64().unwrap_or(0.0);
                            let b = args["b"].as_f64().unwrap_or(0.0);
                            ToolResult::text(format!("{}", a + b))
                        })
                    }),
            )
            .build();

        let mut sdk_mcp_servers = HashMap::new();
        sdk_mcp_servers.insert("test-calc".to_string(), server);

        let options = ClaudeOptions {
            model: Some(TEST_MODEL.to_string()),
            max_turns: Some(3),
            allow_dangerously_skip_permissions: Some(true),
            allowed_tools: Some(vec!["mcp__test-calc__add".to_string()]),
            sdk_mcp_servers: Some(sdk_mcp_servers),
            env: auth_env(),
            ..Default::default()
        };

        let mut handle = query(
            "Use the add tool from the test-calc MCP server to add 3 and 4. \
             Report only the numeric result.",
            options,
        )
        .await
        .expect("Failed to start query");

        let messages = collect_messages(&mut handle.messages).await;

        for msg in &messages {
            match msg {
                SdkMessage::System(_) => eprintln!("  [System]"),
                SdkMessage::Assistant(a) => {
                    let text: String = a
                        .message
                        .content
                        .iter()
                        .filter_map(|b| {
                            if let ContentBlock::Text { text } = b {
                                Some(text.as_str())
                            } else {
                                None
                            }
                        })
                        .collect();
                    eprintln!("  [Assistant] {}", &text[..text.len().min(200)]);
                }
                SdkMessage::Result(ResultMessage::Success(s)) => {
                    eprintln!(
                        "  [Result:Success] turns={} cost=${:.4} text={}",
                        s.num_turns,
                        s.total_cost_usd,
                        &s.result[..s.result.len().min(200)]
                    );
                }
                SdkMessage::Result(ResultMessage::Error(e)) => {
                    eprintln!("  [Result:Error] {:?}", e.errors);
                }
                other => eprintln!("  [{:?}]", std::mem::discriminant(other)),
            }
        }

        assert!(
            messages.iter().any(|m| matches!(m, SdkMessage::Result(_))),
            "Expected Result message"
        );

        assert!(
            tool_was_called.load(Ordering::SeqCst),
            "Expected the add tool to be called"
        );

        let result_text = extract_result_text(&messages);
        let assistant_text = extract_assistant_text(&messages);
        let combined = format!(
            "{} {}",
            assistant_text,
            result_text.as_deref().unwrap_or("")
        );
        assert!(
            combined.contains('7'),
            "Expected response to mention '7', got: {combined}"
        );

        eprintln!("Assistant said: {assistant_text}");
    })
    .await;

    result.expect("Test timed out");
}
