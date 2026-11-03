use super::*;
use crate::agent::harness::content::CanonicalContentBlock;
use crate::agent::harness::source::Role;
use crate::agent::harness::trajectory::{RenderContext, SessionFormat};

fn process() -> PiProcess {
    PiProcess::new(PiConfig {
        config: Arc::new(crate::config::test_support::minimal_config()),
        trajectory: None,
    })
}

#[cfg(unix)]
#[tokio::test]
async fn closing_pi_stops_the_process_group_including_tool_children() {
    use tokio::io::AsyncReadExt;

    let mut child = Command::new("/bin/sh")
        .args(["-c", "/bin/sleep 60 & echo ready; wait"])
        .process_group(0)
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(5), stdout.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(line.trim(), "ready");

    let mut pi = process();
    pi.child = Some(child);
    pi.close().await;

    let mut remaining = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), stdout.read_to_end(&mut remaining))
        .await
        .expect("a surviving tool child kept stdout open")
        .unwrap();
    assert!(pi.ended);
    assert!(pi.child.as_mut().unwrap().try_wait().unwrap().is_some());
}

#[test]
fn queued_prompt_response_does_not_acknowledge_consumption() {
    let mut pi = process();
    pi.inputs.push_back(InputId("first".into()));
    pi.inputs.push_back(InputId("steer".into()));
    pi.observe(
        &json!({"type":"response","id":"steer","success":true,"data":{"disposition":"queued"}}),
    );
    assert!(
        !pi.pending
            .iter()
            .any(|e| matches!(e, ProcessEvent::InputAccepted { .. }))
    );
    pi.observe(&json!({"type":"message_start","message":{"role":"user","content":"first"}}));
    pi.observe(&json!({"type":"message_start","message":{"role":"user","content":"steer"}}));
    let accepted: Vec<_> = pi
        .pending
        .iter()
        .filter_map(|e| {
            if let ProcessEvent::InputAccepted { input } = e {
                Some(input.0.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(accepted, ["first", "steer"]);
}

#[test]
fn agent_end_is_not_terminal_and_usage_counts_only_new_messages() {
    let mut pi = process();
    pi.observe(&json!({"type":"agent_end"}));
    assert!(!pi.ended);
    assert!(
        !pi.pending
            .iter()
            .any(|e| matches!(e, ProcessEvent::Terminated { .. }))
    );
    for _ in 0..2 {
        pi.observe(&json!({"type":"message_end","message":{"role":"assistant","content":[],"usage":{"input":10,"output":3,"cacheRead":5,"cacheWrite":2,"cost":{"total":0.1}}}}));
    }
    assert_eq!(pi.totals.input_tokens, 20);
    assert_eq!(pi.totals.output_tokens, 6);
    assert_eq!(pi.totals.cache_read_input_tokens, 10);
    assert_eq!(pi.totals.total_cost_usd, Some(0.2));
}

#[test]
fn native_session_preserves_tool_identity_and_fork_payload_bytes() {
    let line = r#"{ "type":"message", "id":"entry", "message":{"role":"toolResult","toolCallId":"call","isError":true,"content":[{"type":"text","text":"failed"}]}}"#;
    let read = PiSessionFormat.read_line(line);
    assert_eq!(read.content.role, Some(Role::User));
    assert!(
        matches!(&read.content.blocks[0],CanonicalContentBlock::ToolResult{tool_use_id,is_error:Some(true),..} if tool_use_id=="call")
    );
    assert_eq!(PiSessionFormat.retarget(line, "fork"), line);
}

#[test]
fn switched_history_keeps_roles_and_links_entries_after_the_header() {
    let items=["user","assistant"].map(|role|PiSessionFormat.read_line(&json!({"type":"message","message":{"role":role,"content":[{"type":"text","text":role}]}}).to_string()).content);
    let lines = PiSessionFormat.render(
        &items,
        &RenderContext {
            harness_session_id: "session",
            cwd: std::path::Path::new("/workspace"),
            model: Some("anthropic/claude-sonnet-5-5"),
            prior: &[],
            timestamp: "2026-01-01T00:00:00Z",
        },
    );
    let values: Vec<Value> = lines
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(values[1]["parentId"], Value::Null);
    assert_eq!(values[2]["parentId"], values[1]["id"]);
    assert_eq!(values[1]["message"]["role"], "user");
    assert_eq!(values[2]["message"]["role"], "assistant");
}

#[test]
fn the_openrouter_provider_speaks_responses_to_the_gateway_and_declares_its_model() {
    let provider = openrouter_provider("http://backend:8080/", "!cat t", "mistralai/mistral-large");

    assert_eq!(provider["baseUrl"], "http://backend:8080/llm/openai/v1");
    assert_eq!(provider["api"], "openai-responses");
    assert_eq!(provider["models"][0]["id"], "mistralai/mistral-large");
}
