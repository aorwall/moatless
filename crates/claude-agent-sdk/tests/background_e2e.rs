#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! E2E reproduction: what happens to a `run_in_background` Bash job when the
//! one-shot `claude -p` (subprocess) process exits at the end of a turn.
//!
//! Motivation: in the sandbox's default subprocess dispatch, each turn is a
//! separate `claude --print` process. Claude Code documents that after it
//! prints the final result and stdin closes, any background Bash shell is given
//! a short grace period and then **terminated** so the `-p` process can exit
//! cleanly (see https://code.claude.com/docs/en/headless and
//! anthropics/claude-code#65498). This test proves that empirically: a
//! background writer that should run for ~60s stops growing within a few
//! seconds of the turn's `Result` — i.e. it was killed, not left running.
//!
//! Run:
//!   CLAUDE_SDK_E2E=1 cargo test -p claude-agent-sdk --test background_e2e \
//!       -- --ignored --nocapture

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use claude_agent_sdk::types::messages::ResultMessage;
use claude_agent_sdk::{query, query_tmux, ClaudeOptions, SdkMessage};

const TEST_MODEL: &str = "claude-haiku-4-5-20251001";

/// The background writer runs for this many one-second iterations. Much longer
/// than any plausible post-result grace period, so "still running" vs "killed"
/// is unambiguous.
const WRITER_ITERS: u32 = 60;

/// How long we watch the marker file after the turn's Result before concluding.
const OBSERVE_AFTER_RESULT: Duration = Duration::from_secs(14);

fn skip_unless_e2e() -> bool {
    if std::env::var("CLAUDE_SDK_E2E").as_deref() != Ok("1") {
        eprintln!("Skipping background E2E test (set CLAUDE_SDK_E2E=1 to run)");
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
    if env.is_empty() {
        None
    } else {
        Some(env)
    }
}

fn fresh_cwd() -> PathBuf {
    let root = std::env::temp_dir().join("claude-sdk-bg-e2e");
    std::fs::create_dir_all(&root).expect("create root");
    let cwd = root.join(uuid::Uuid::new_v4().simple().to_string());
    std::fs::create_dir_all(&cwd).expect("create cwd");
    cwd
}

fn line_count(path: &PathBuf) -> usize {
    std::fs::read_to_string(path).map_or(0, |s| s.lines().count())
}

#[tokio::test]
#[ignore = "requires claude CLI + API key (CLAUDE_SDK_E2E=1)"]
async fn background_bash_is_killed_when_print_process_exits() {
    if skip_unless_e2e() {
        return;
    }

    let cwd = fresh_cwd();
    let marker = cwd.join("bg.log");
    // Absolute path so the backgrounded shell writes to a file we can watch
    // regardless of its own cwd.
    let marker_str = marker.to_string_lossy().to_string();

    // A writer that appends one line per second for WRITER_ITERS seconds.
    let bg_cmd = format!(
        "for i in $(seq 1 {WRITER_ITERS}); do echo \"line $i\" >> {marker_str}; sleep 1; done"
    );

    let prompt = format!(
        "Use the Bash tool with run_in_background set to true to start EXACTLY this command, \
         then immediately end your turn. Do NOT wait for it, do NOT poll it, do NOT read its \
         output, do NOT run anything else. Command: {bg_cmd}"
    );

    let options = ClaudeOptions {
        model: Some(TEST_MODEL.to_string()),
        // 2 turns: one for the Bash(run_in_background) tool call, one for the
        // model's closing text — so the Result is a clean success rather than
        // an error_max_turns after the tool use.
        max_turns: Some(2),
        allow_dangerously_skip_permissions: Some(true),
        allowed_tools: Some(vec!["Bash".to_string()]),
        cwd: Some(cwd.to_string_lossy().into_owned()),
        env: auth_env(),
        ..Default::default()
    };

    // Keep the handle alive for the whole test so the CLI child exits on its
    // own (natural post-result cleanup) rather than being SIGKILLed by us
    // dropping the transport.
    let mut handle = query(&prompt, options).await.expect("failed to start query");

    let started = Instant::now();
    let mut saw_result = false;
    while let Some(msg) = handle.messages.recv().await {
        match msg {
            Ok(SdkMessage::Result(r)) => {
                let ok = matches!(r, ResultMessage::Success(_));
                eprintln!(
                    "[{:>5.1}s] Result received (success={ok}); lines so far = {}",
                    started.elapsed().as_secs_f64(),
                    line_count(&marker)
                );
                saw_result = true;
                break;
            }
            Ok(_) => {}
            Err(e) => panic!("error receiving message: {e}"),
        }
    }
    assert!(saw_result, "never received a Result message");

    let count_at_result = line_count(&marker);
    assert!(
        count_at_result < WRITER_ITERS as usize,
        "writer already completed before the turn ended ({count_at_result} lines) — it was not \
         actually backgrounded; adjust the prompt"
    );

    // Watch the marker file after the Result. Record the timeline and the
    // instant growth stops.
    let result_at = Instant::now();
    let mut last = count_at_result;
    let mut last_growth_at = result_at;
    loop {
        let elapsed = result_at.elapsed();
        if elapsed >= OBSERVE_AFTER_RESULT {
            break;
        }
        let now = line_count(&marker);
        if now != last {
            eprintln!(
                "[+{:>4.1}s after result] lines = {now}",
                elapsed.as_secs_f64()
            );
            last = now;
            last_growth_at = Instant::now();
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let final_count = line_count(&marker);
    let quiet_for = last_growth_at.elapsed().as_secs_f64();
    eprintln!(
        "final lines = {final_count} (writer target = {WRITER_ITERS}); \
         growth stopped {quiet_for:.1}s before end of observation window"
    );

    // The writer targets WRITER_ITERS (~60) lines over ~60s. If Claude Code's
    // post-result cleanup killed it, the file stops growing within a few
    // seconds of the Result and never reaches the target.
    assert!(
        final_count < WRITER_ITERS as usize,
        "background writer ran to completion ({final_count} lines) — it was NOT killed after the \
         print process exited (unexpected in subprocess/--print mode)"
    );
    assert!(
        quiet_for >= 3.0,
        "background writer was still growing at the end of the observation window \
         (final={final_count}) — expected it to be killed shortly after Result"
    );

    eprintln!(
        "REPRODUCED: run_in_background job was killed ~{:.0}s after the turn's Result \
         (wrote {final_count}/{WRITER_ITERS} lines).",
        (OBSERVE_AFTER_RESULT.as_secs_f64() - quiet_for)
    );
}

/// Contrast: under the tmux backend the CLI (TUI) stays alive after the turn's
/// Result, so the same `run_in_background` job keeps running. This is the
/// behaviour subprocess mode lacks — and the reason the sandbox skips the
/// background-completion watch in tmux mode (the CLI delivers its own
/// completion notification there).
///
/// Requires `CLAUDE_TMUX_E2E=1` and the `tmux` binary on PATH.
#[tokio::test]
#[ignore = "requires claude CLI + API key + tmux (CLAUDE_TMUX_E2E=1)"]
async fn background_bash_survives_under_tmux() {
    if std::env::var("CLAUDE_TMUX_E2E").as_deref() != Ok("1") {
        eprintln!("Skipping tmux background E2E test (set CLAUDE_TMUX_E2E=1 to run)");
        return;
    }
    if std::process::Command::new("tmux").arg("-V").output().is_err() {
        eprintln!("Skipping tmux background E2E test: `tmux` not on PATH");
        return;
    }

    let cwd = fresh_cwd();
    let marker = cwd.join("bg.log");
    let marker_str = marker.to_string_lossy().to_string();
    let bg_cmd = format!(
        "for i in $(seq 1 {WRITER_ITERS}); do echo \"line $i\" >> {marker_str}; sleep 1; done"
    );
    let prompt = format!(
        "Use the Bash tool with run_in_background set to true to start EXACTLY this command, \
         then immediately end your turn. Do NOT wait for it. Command: {bg_cmd}"
    );

    let options = ClaudeOptions {
        model: Some(TEST_MODEL.to_string()),
        max_turns: Some(2),
        allow_dangerously_skip_permissions: Some(true),
        allowed_tools: Some(vec!["Bash".to_string()]),
        cwd: Some(cwd.to_string_lossy().into_owned()),
        env: auth_env(),
        ..Default::default()
    };

    // Hold the handle for the whole test so the tmux session stays alive.
    let mut handle = query_tmux(&prompt, options)
        .await
        .expect("failed to start tmux query");

    let mut saw_result = false;
    while let Some(msg) = handle.messages.recv().await {
        if let Ok(SdkMessage::Result(_)) = msg {
            saw_result = true;
            break;
        }
    }
    assert!(saw_result, "never received a Result message (tmux)");

    let count_at_result = line_count(&marker);
    eprintln!("[tmux] lines at result = {count_at_result}");

    // Watch for continued growth after the Result.
    let result_at = Instant::now();
    let mut last = count_at_result;
    let mut last_growth_at = result_at;
    while result_at.elapsed() < OBSERVE_AFTER_RESULT {
        let now = line_count(&marker);
        if now != last {
            eprintln!(
                "[tmux +{:>4.1}s after result] lines = {now}",
                result_at.elapsed().as_secs_f64()
            );
            last = now;
            last_growth_at = Instant::now();
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let final_count = line_count(&marker);
    let quiet_for = last_growth_at.elapsed().as_secs_f64();
    eprintln!("[tmux] final lines = {final_count}; quiet_for={quiet_for:.1}s");

    // The persistent TUI keeps the job alive: the file should still be growing
    // near the end of the observation window.
    assert!(
        final_count > count_at_result,
        "background writer made no progress after Result under tmux (final={final_count}) — \
         expected it to keep running"
    );
    assert!(
        quiet_for < 3.0,
        "background writer stopped growing under tmux (final={final_count}) — expected the \
         persistent TUI to keep it alive"
    );

    eprintln!(
        "CONTRAST: under tmux the run_in_background job kept running after Result \
         ({final_count} lines and counting)."
    );
}
