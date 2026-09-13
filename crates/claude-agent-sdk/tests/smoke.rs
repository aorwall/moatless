#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stderr,
    // Timeout consts read more clearly as `from_secs(..)` than the lint's
    // `from_mins` rewrite.
    clippy::duration_suboptimal_units
)]
//! Smoke test for claude-agent-sdk.
//!
//! Verifies the `claude` CLI on PATH matches `CLAUDE_CLI_VERSION` in
//! `docker/Dockerfile.sandbox-base` (catches pin drift), a single roundtrip
//! System → Assistant → Result with matching `session_id`s, and that per-turn
//! `--resume <session_id>` recalls context across two turns — how the sandbox
//! dispatches multi-turn tasks. Gated on `CLAUDE_SDK_E2E=1`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::time::timeout;

use claude_agent_sdk::api::messages::ResultMessage;
use claude_agent_sdk::{ClaudeOptions, SdkMessage, query};

/// MUST match `CLAUDE_CLI_VERSION` in `docker/Dockerfile.sandbox-base`.
/// Bump this when the Dockerfile pin is bumped.
const EXPECTED_CLAUDE_CLI_VERSION: &str = "2.1.270";

const TEST_MODEL: &str = "claude-haiku-4-5-20251001";
const TEST_TIMEOUT: Duration = Duration::from_secs(60);
const RESUME_TEST_TIMEOUT: Duration = Duration::from_secs(90);

fn skip_unless_e2e() -> bool {
    if std::env::var("CLAUDE_SDK_E2E").as_deref() != Ok("1") {
        eprintln!("Skipping smoke test (set CLAUDE_SDK_E2E=1 to run)");
        return true;
    }
    false
}

fn auth_env() -> Option<HashMap<String, String>> {
    let mut env = HashMap::new();
    if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        env.insert("CLAUDE_CODE_OAUTH_TOKEN".to_string(), token);
    }
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        env.insert("ANTHROPIC_API_KEY".to_string(), key);
    }
    if env.is_empty() { None } else { Some(env) }
}

fn fresh_cwd(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join("claude-sdk-smoke");
    std::fs::create_dir_all(&root).expect("create smoke root");
    let cwd = root.join(format!("{name}-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&cwd).expect("create fresh test cwd");
    cwd
}

fn base_options(cwd: &Path) -> ClaudeOptions {
    ClaudeOptions {
        model: Some(TEST_MODEL.to_string()),
        max_turns: Some(1),
        allow_dangerously_skip_permissions: Some(true),
        cwd: Some(cwd.to_string_lossy().into_owned()),
        env: auth_env(),
        ..Default::default()
    }
}

async fn run_one_shot(prompt: &str, options: ClaudeOptions) -> Vec<SdkMessage> {
    let mut handle = query(prompt, options).await.expect("failed to start query");
    let mut messages = Vec::new();
    while let Some(msg) = handle.messages.recv().await {
        let msg = msg.expect("message error");
        let done = matches!(&msg, SdkMessage::Result(_));
        messages.push(msg);
        if done {
            break;
        }
    }
    messages
}

fn assert_cli_version_matches_pin() {
    let output = std::process::Command::new("claude")
        .arg("--version")
        .output()
        .expect("`claude` binary not found on PATH");
    let version_line = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert!(
        version_line.starts_with(EXPECTED_CLAUDE_CLI_VERSION),
        "claude --version mismatch: expected `{EXPECTED_CLAUDE_CLI_VERSION}*`, got `{version_line}` \
         (update EXPECTED_CLAUDE_CLI_VERSION in tests/smoke.rs and \
         CLAUDE_CLI_VERSION in docker/Dockerfile.sandbox-base together)"
    );
}

#[tokio::test]
#[ignore = "spawns claude CLI + makes API calls; gated on CLAUDE_SDK_E2E=1"]
async fn test_smoke_simple_query() {
    if skip_unless_e2e() {
        return;
    }
    assert_cli_version_matches_pin();

    timeout(TEST_TIMEOUT, async {
        let cwd = fresh_cwd("simple");
        let messages = run_one_shot(
            "What is 2+2? Reply with only the number.",
            base_options(&cwd),
        )
        .await;

        let sys_id = messages
            .iter()
            .find_map(|m| match m {
                SdkMessage::System(init) => Some(init.session_id.clone()),
                _ => None,
            })
            .expect("missing System init");

        let (res_id, result_text) = messages
            .iter()
            .find_map(|m| match m {
                SdkMessage::Result(ResultMessage::Success(s)) => {
                    Some((s.session_id.clone(), s.result.clone()))
                }
                SdkMessage::Result(ResultMessage::Error(e)) => {
                    panic!("query errored: {:?}", e.errors)
                }
                _ => None,
            })
            .expect("missing Result(Success)");

        assert!(
            messages
                .iter()
                .any(|m| matches!(m, SdkMessage::Assistant(_))),
            "missing Assistant message"
        );
        assert_eq!(sys_id, res_id, "System.session_id != Result.session_id");
        assert!(!sys_id.is_empty(), "session_id is empty");
        assert!(
            result_text.contains('4'),
            "result missing '4': {result_text}"
        );
    })
    .await
    .expect("simple_query timed out");
}

#[tokio::test]
#[ignore = "spawns claude CLI + makes API calls; gated on CLAUDE_SDK_E2E=1"]
async fn test_smoke_session_resume() {
    if skip_unless_e2e() {
        return;
    }
    assert_cli_version_matches_pin();

    timeout(RESUME_TEST_TIMEOUT, async {
        let cwd = fresh_cwd("resume");

        let t1 = run_one_shot(
            "Write a Rust function `get_answer()` that returns the &str \"blue elephant\". \
             Just show the function, nothing else.",
            base_options(&cwd),
        )
        .await;

        let session_id = t1
            .iter()
            .find_map(|m| match m {
                SdkMessage::System(init) => Some(init.session_id.clone()),
                _ => None,
            })
            .expect("turn 1 missing System init");

        let t1_text = t1
            .iter()
            .find_map(|m| match m {
                SdkMessage::Result(ResultMessage::Success(s)) => Some(s.result.clone()),
                SdkMessage::Result(ResultMessage::Error(e)) => {
                    panic!("turn 1 errored: {:?}", e.errors)
                }
                _ => None,
            })
            .expect("turn 1 missing Result(Success)");
        assert!(
            t1_text.to_lowercase().contains("blue elephant"),
            "turn 1 missing phrase: {t1_text}"
        );

        let t2_options = ClaudeOptions {
            resume: Some(session_id.clone()),
            ..base_options(&cwd)
        };
        let t2 = run_one_shot(
            "What string does the `get_answer()` function return? Reply with just the string.",
            t2_options,
        )
        .await;

        let t2_text = t2
            .iter()
            .find_map(|m| match m {
                SdkMessage::Result(ResultMessage::Success(s)) => Some(s.result.clone()),
                SdkMessage::Result(ResultMessage::Error(e)) => {
                    panic!("turn 2 errored: {:?}", e.errors)
                }
                _ => None,
            })
            .expect("turn 2 missing Result(Success)");
        assert!(
            t2_text.to_lowercase().contains("blue elephant"),
            "turn 2 should recall 'blue elephant', got: {t2_text}"
        );
    })
    .await
    .expect("session_resume timed out");
}
