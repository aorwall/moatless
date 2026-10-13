//! The tmux TUI over a pod's lifetime: many turns through one session, and
//! resuming that session after the TUI dies.

use crate::harness::{
    TEST_MODEL, TMUX_LONG_TEST_TIMEOUT, auth_env, extract_assistant_text, extract_result_text,
    fresh_test_cwd, init_tracing,
};
use claude_agent_sdk::ClaudeClient;
use claude_agent_sdk::ClaudeOptions;
use claude_agent_sdk::SdkMessage;
use claude_agent_sdk::api::messages::ResultMessage;
use std::time::Duration;
use tokio::time::timeout;

/// Drives the tmux backend the way `moatless-sandbox` does: one
/// `ClaudeClient::new_tmux` for the lifetime of the pod, then repeated `send`
/// turns into the same TUI.
#[tokio::test]
#[ignore = "tmux + claude CLI required; gated on CLAUDE_TMUX_E2E=1"]
async fn test_long_running_three_turns_tmux() {
    skip_unless_tmux_e2e!();
    init_tracing();

    let cwd = fresh_test_cwd("long-running");
    let options = ClaudeOptions {
        model: Some(TEST_MODEL.to_string()),
        max_turns: Some(1),
        allow_dangerously_skip_permissions: Some(true),
        cwd: Some(cwd.to_string_lossy().into_owned()),
        env: auth_env(),
        ..Default::default()
    };

    let test_body = async {
        let mut client = ClaudeClient::new_tmux(options)
            .await
            .expect("Failed to create tmux ClaudeClient");

        let mut session_id = String::new();

        client
            .send(
                "Remember: my favorite color is teal. Confirm in <=5 words.",
                "",
            )
            .await
            .expect("turn 1 send");

        let mut turn1 = Vec::new();
        while let Some(msg) = client.recv().await {
            let msg = msg.expect("turn 1 recv");
            if let SdkMessage::System(ref init) = msg {
                session_id = init.session_id.clone();
            }
            let is_result = matches!(&msg, SdkMessage::Result(_));
            turn1.push(msg);
            if is_result {
                break;
            }
        }
        assert!(!session_id.is_empty(), "turn 1 must yield session_id");
        eprintln!(
            "[turn 1] session_id={session_id} result={:?}",
            extract_result_text(&turn1).unwrap_or_default()
        );

        client
            .send(
                "Also remember: my favorite number is 17. What color did I tell you?",
                &session_id,
            )
            .await
            .expect("turn 2 send");

        let mut turn2 = Vec::new();
        while let Some(msg) = client.recv().await {
            let msg = msg.expect("turn 2 recv");
            let is_result = matches!(&msg, SdkMessage::Result(_));
            turn2.push(msg);
            if is_result {
                break;
            }
        }
        let turn2_combined = format!(
            "{} {}",
            extract_assistant_text(&turn2),
            extract_result_text(&turn2).unwrap_or_default()
        );
        eprintln!("[turn 2] combined={turn2_combined}");
        assert!(
            turn2_combined.to_lowercase().contains("teal"),
            "turn 2 should recall 'teal' from turn 1, got: {turn2_combined}"
        );

        client
            .send(
                "Repeat both: my color and my number. Just the two values.",
                &session_id,
            )
            .await
            .expect("turn 3 send");

        let mut turn3 = Vec::new();
        while let Some(msg) = client.recv().await {
            let msg = msg.expect("turn 3 recv");
            let is_result = matches!(&msg, SdkMessage::Result(_));
            turn3.push(msg);
            if is_result {
                break;
            }
        }
        let turn3_combined = format!(
            "{} {}",
            extract_assistant_text(&turn3),
            extract_result_text(&turn3).unwrap_or_default()
        );
        eprintln!("[turn 3] combined={turn3_combined}");
        let lower = turn3_combined.to_lowercase();
        assert!(
            lower.contains("teal"),
            "turn 3 should recall 'teal', got: {turn3_combined}"
        );
        assert!(
            lower.contains("17"),
            "turn 3 should recall '17', got: {turn3_combined}"
        );

        let session_ids: Vec<&str> = turn1
            .iter()
            .chain(turn2.iter())
            .chain(turn3.iter())
            .filter_map(|m| match m {
                SdkMessage::Result(ResultMessage::Success(s)) => Some(s.session_id.as_str()),
                SdkMessage::Result(ResultMessage::Error(e)) => Some(e.session_id.as_str()),
                _ => None,
            })
            .collect();
        for sid in &session_ids {
            assert_eq!(
                *sid, session_id,
                "all turns must share the same session_id (proves single TUI)"
            );
        }
        eprintln!(
            "All 3 turns landed on the same session_id ({} Result messages observed)",
            session_ids.len()
        );

        client.close();
    };

    timeout(TMUX_LONG_TEST_TIMEOUT, test_body)
        .await
        .expect("Test timed out");
}

/// The path a restarted pod takes: the in-pod TUI dies, the sandbox comes back
/// with the previous `session_id`, and a fresh client resuming it under the
/// same `cwd` still recalls the conversation.
#[tokio::test]
#[ignore = "tmux + claude CLI required; gated on CLAUDE_TMUX_E2E=1"]
async fn test_sandbox_restart_resume_tmux() {
    skip_unless_tmux_e2e!();
    init_tracing();

    let cwd = fresh_test_cwd("sandbox-restart");
    let cwd_str = cwd.to_string_lossy().into_owned();
    let base_options = || ClaudeOptions {
        model: Some(TEST_MODEL.to_string()),
        max_turns: Some(1),
        allow_dangerously_skip_permissions: Some(true),
        cwd: Some(cwd_str.clone()),
        env: auth_env(),
        ..Default::default()
    };

    let test_body = async {
        let mut client = ClaudeClient::new_tmux(base_options())
            .await
            .expect("Failed to create initial tmux ClaudeClient");

        let mut session_id = String::new();

        client
            .send(
                "Remember: my favorite color is teal. Confirm in <=5 words.",
                "",
            )
            .await
            .expect("turn 1 send");
        let mut turn1 = Vec::new();
        while let Some(msg) = client.recv().await {
            let msg = msg.expect("turn 1 recv");
            if let SdkMessage::System(ref init) = msg {
                session_id = init.session_id.clone();
            }
            let is_result = matches!(&msg, SdkMessage::Result(_));
            turn1.push(msg);
            if is_result {
                break;
            }
        }
        assert!(!session_id.is_empty(), "must have session_id after turn 1");
        eprintln!("[pre-restart turn 1] session_id={session_id}");

        client
            .send(
                "Also remember: my favorite number is 17. Confirm in <=5 words.",
                &session_id,
            )
            .await
            .expect("turn 2 send");
        let mut turn2 = Vec::new();
        while let Some(msg) = client.recv().await {
            let msg = msg.expect("turn 2 recv");
            let is_result = matches!(&msg, SdkMessage::Result(_));
            turn2.push(msg);
            if is_result {
                break;
            }
        }
        eprintln!("[pre-restart turn 2] done");

        // ── Simulated restart: tear down the TUI ──────────────────
        // Dropping the client cancels the supervisor's CancellationToken,
        // which kills the tmux session and escalates to SIGKILL if needed.
        client.close();
        drop(client);
        eprintln!("[restart] TUI torn down — sleeping 500ms to let kill settle");
        tokio::time::sleep(Duration::from_millis(500)).await;

        // ── Post-restart: fresh tmux client with --resume ─────────
        // Same cwd so the project_dir / JSONL filename resolves identically.
        let resume_options = ClaudeOptions {
            resume: Some(session_id.clone()),
            ..base_options()
        };
        let mut resumed = ClaudeClient::new_tmux(resume_options)
            .await
            .expect("Failed to create resumed tmux ClaudeClient");

        resumed
            .send(
                "Repeat my color and my number. Just those two values, comma-separated.",
                &session_id,
            )
            .await
            .expect("resumed send");

        let mut turn3 = Vec::new();
        while let Some(msg) = resumed.recv().await {
            let msg = msg.expect("resumed recv");
            let is_result = matches!(&msg, SdkMessage::Result(_));
            turn3.push(msg);
            if is_result {
                break;
            }
        }

        let combined = format!(
            "{} {}",
            extract_assistant_text(&turn3),
            extract_result_text(&turn3).unwrap_or_default()
        );
        eprintln!("[post-restart] combined={combined}");
        let lower = combined.to_lowercase();
        assert!(
            lower.contains("teal"),
            "resumed session must recall 'teal' from pre-restart turn 1; got: {combined}"
        );
        assert!(
            lower.contains("17"),
            "resumed session must recall '17' from pre-restart turn 2; got: {combined}"
        );

        let post_restart_session_ids: Vec<&str> = turn3
            .iter()
            .filter_map(|m| match m {
                SdkMessage::Result(ResultMessage::Success(s)) => Some(s.session_id.as_str()),
                SdkMessage::Result(ResultMessage::Error(e)) => Some(e.session_id.as_str()),
                _ => None,
            })
            .collect();
        for sid in &post_restart_session_ids {
            assert_eq!(
                *sid, session_id,
                "post-restart turn must keep the same session_id"
            );
        }
        eprintln!("Resume across restart preserved session_id {session_id}, recalled all facts.");

        resumed.close();
    };

    timeout(TMUX_LONG_TEST_TIMEOUT, test_body)
        .await
        .expect("Test timed out");
}
