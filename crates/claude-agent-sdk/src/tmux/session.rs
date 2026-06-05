//! Manage a detached tmux session running the interactive Claude TUI.
//!
//! Lifecycle:
//! 1. Snapshot existing `*.jsonl` files in the project directory (caller does
//!    this BEFORE spawn, so we know which files are new afterwards).
//! 2. `tmux new-session -d -s <name> -x 220 -y 50 -- env -u CLAUDECODE claude <args...>`.
//! 3. **The TUI does not write a JSONL until the first user message is
//!    submitted.** The caller must paste a prompt first, then call
//!    [`discover_new_jsonl`] to wait for the file to appear.
//! 4. Hand control to the tailer.
//!
//! Cleanup runs through `kill()` (called by the supervisor task on
//! cancellation) and a best-effort sync kill in `Drop` as a safety net.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tracing::{debug, info, warn};

use crate::error::{Result, SdkError};
use crate::types::options::ClaudeOptions;

use super::args::build_tmux_cli_args;

/// Default tmux window dimensions for the detached session.
const TMUX_WIDTH: u32 = 220;
const TMUX_HEIGHT: u32 = 50;

/// One spawned tmux + claude session.
///
/// JSONL discovery is intentionally NOT part of this struct — the TUI defers
/// creating the transcript file until the first user message is submitted, so
/// the orchestration in `mod.rs` must paste a prompt before discovering.
pub(crate) struct TmuxSession {
    pub(crate) session_name: String,
}

impl TmuxSession {
    /// Spawn a detached tmux session running `claude` with the given options.
    /// Does not wait for the JSONL transcript — see [`discover_new_jsonl`].
    ///
    /// `settings_json` is the harness-owned inline `--settings` JSON (or
    /// `None` to skip the `--settings` flag entirely).
    pub(crate) async fn spawn(
        options: &ClaudeOptions,
        settings_json: Option<&str>,
    ) -> Result<Self> {
        let cli_path = resolve_cli_path(options);

        let session_name = format!(
            "claude-sdk-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        );
        let args = build_tmux_cli_args(options, settings_json);
        info!(
            session_name = %session_name,
            cli_path = %cli_path,
            args = ?args,
            cwd = ?options.cwd,
            "Spawning tmux session for Claude TUI"
        );

        let mut cmd = tokio::process::Command::new("tmux");
        cmd.arg("new-session")
            .arg("-d")
            .arg("-s")
            .arg(&session_name)
            .arg("-x")
            .arg(TMUX_WIDTH.to_string())
            .arg("-y")
            .arg(TMUX_HEIGHT.to_string());
        if let Some(cwd) = &options.cwd {
            cmd.arg("-c").arg(cwd);
        }
        cmd.arg("--");
        // `env -u CLAUDECODE` mirrors transport/subprocess.rs:46 — claude
        // refuses to start when it sees CLAUDECODE set.
        cmd.arg("env").arg("-u").arg("CLAUDECODE");
        if let Some(env) = &options.env {
            for (k, v) in env {
                cmd.arg(format!("{k}={v}"));
            }
        }
        cmd.arg(&cli_path);
        for a in &args {
            cmd.arg(a);
        }

        let status = cmd.status().await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                SdkError::TmuxNotFound
            } else {
                SdkError::SpawnError(e)
            }
        })?;
        if !status.success() {
            return Err(SdkError::Internal(format!(
                "tmux new-session exited with status {status:?}"
            )));
        }

        debug!(session_name = %session_name, "tmux session spawned");
        Ok(Self { session_name })
    }

    /// Kill the tmux session (and the claude process inside).
    ///
    /// Escalation, mirroring `claude-p`'s sequence:
    /// 1. `tmux kill-session` (graceful — sends SIGHUP via tmux's pane reaper).
    /// 2. Poll `tmux has-session` every 50ms for up to 200ms.
    /// 3. If still alive, grab the pane PID and send SIGKILL directly.
    pub(crate) async fn kill(&self) {
        debug!(session_name = %self.session_name, "Killing tmux session");

        // Capture the pane PID BEFORE killing the session — once tmux exits,
        // `display-message` returns nothing.
        let pane_pid = self.pane_pid().await;

        let result = tokio::process::Command::new("tmux")
            .args(["kill-session", "-t", &self.session_name])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
        if let Err(e) = result {
            warn!(error = %e, session_name = %self.session_name, "tmux kill-session failed");
        }

        // Wait up to 200ms (4 × 50ms) for the session to actually go away.
        for _ in 0..4 {
            if !self.is_alive().await {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        // Still alive — escalate. Send SIGKILL to the pane's pid (the claude
        // process). `kill -KILL` is POSIX-standard; no `unsafe` required.
        if let Some(pid) = pane_pid {
            debug!(pid, session_name = %self.session_name, "Escalating to SIGKILL on pane PID");
            let kill_result = tokio::process::Command::new("kill")
                .args(["-KILL", &pid.to_string()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await;
            if let Err(e) = kill_result {
                warn!(error = %e, pid, "kill -KILL failed");
            }
        } else {
            warn!(
                session_name = %self.session_name,
                "tmux session still alive after kill-session but no pane PID captured"
            );
        }
    }

    /// Capture the pane PID of the tmux session, used by [`Self::kill`] for
    /// the SIGKILL escalation step. Returns `None` if the session has already
    /// exited or the PID couldn't be parsed.
    async fn pane_pid(&self) -> Option<u32> {
        let out = tokio::process::Command::new("tmux")
            .args([
                "display-message",
                "-p",
                "-t",
                &self.session_name,
                "#{pane_pid}",
            ])
            .stderr(std::process::Stdio::null())
            .output()
            .await
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = std::str::from_utf8(&out.stdout).ok()?.trim();
        s.parse::<u32>().ok()
    }

    /// Return true if the tmux session is still alive.
    pub(crate) async fn is_alive(&self) -> bool {
        tokio::process::Command::new("tmux")
            .args(["has-session", "-t", &self.session_name])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

impl Drop for TmuxSession {
    fn drop(&mut self) {
        // Best-effort synchronous kill in case the async cleanup path didn't
        // run (e.g. supervisor task panicked). Won't block long.
        let _ = std::process::Command::new("tmux")
            .args(["kill-session", "-t", &self.session_name])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// Resolve the `claude` CLI binary path the same way the subprocess backend
/// does.
fn resolve_cli_path(options: &ClaudeOptions) -> String {
    if let Some(p) = &options.cli_path {
        return p.clone();
    }
    if let Ok(output) = std::process::Command::new("which").arg("claude").output() {
        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let default = format!("{home}/.claude/local/claude");
    if std::path::Path::new(&default).exists() {
        return default;
    }
    "claude".to_string()
}

/// Resolve `<claude-config-dir>/projects/<dashed-cwd>/`.
///
/// The config dir is resolved by the shared [`crate::resolve_claude_config_dir`]
/// helper, which mirrors the Claude CLI's precedence: `CLAUDE_CONFIG_DIR`, then
/// its documented `CLAUDE_HOME` alias, then `$HOME/.claude`. The TUI encodes a
/// cwd by replacing `/` with `-` (e.g. `/Users/x/y` becomes `-Users-x-y`).
pub(crate) fn resolve_project_dir(options: &ClaudeOptions) -> Result<PathBuf> {
    let cwd_str = if let Some(c) = &options.cwd {
        std::path::PathBuf::from(c)
            .canonicalize()
            .map_err(|e| SdkError::Internal(format!("Failed to canonicalize cwd {c}: {e}")))?
            .to_string_lossy()
            .into_owned()
    } else {
        std::env::current_dir()
            .map_err(|e| SdkError::Internal(format!("std::env::current_dir failed: {e}")))?
            .to_string_lossy()
            .into_owned()
    };

    let dashed = cwd_str.replace('/', "-");

    // Claude writes transcripts under its config dir. We resolve it via the
    // shared helper that mirrors the CLI (`CLAUDE_CONFIG_DIR`, then its
    // `CLAUDE_HOME` alias, then `$HOME/.claude`). The sandbox sets
    // `CLAUDE_CONFIG_DIR` (e.g. `/opt/moatless/.claude`) while `$HOME` is a
    // different path, so honouring it here keeps JSONL discovery pointed at the
    // directory the CLI actually writes to.
    let home = std::env::var("HOME").unwrap_or_default();
    let config_dir = crate::config_dir::resolve_claude_config_dir(&home);

    Ok(config_dir.join("projects").join(dashed))
}

pub(crate) async fn list_jsonl_stems(dir: &Path) -> Result<HashSet<String>> {
    let mut set = HashSet::new();
    let mut read = match tokio::fs::read_dir(dir).await {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(set),
        Err(e) => {
            return Err(SdkError::Internal(format!(
                "Failed to read project dir {}: {e}",
                dir.display()
            )))
        }
    };
    while let Some(entry) = read
        .next_entry()
        .await
        .map_err(|e| SdkError::Internal(format!("read_dir.next_entry failed: {e}")))?
    {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                set.insert(stem.to_string());
            }
        }
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The env-var precedence itself is covered by `crate::config_dir` tests;
    // here we only assert that `resolve_project_dir` joins `projects/<dashed-cwd>`
    // under the resolved config dir. Mutating process-global env vars must run
    // serially, so this is a single test.
    #[test]
    fn project_dir_joins_projects_under_config_dir() {
        let _guard = crate::config_dir::ENV_LOCK.lock().expect("env lock");
        // Use an existing directory so canonicalize() succeeds — tempdir works.
        let tmp = std::env::temp_dir();
        let canonical = tmp.canonicalize().expect("temp_dir should canonicalize");
        let cwd_str = canonical.to_string_lossy().into_owned();
        let dashed = cwd_str.replace('/', "-");
        let options = ClaudeOptions {
            cwd: Some(cwd_str.clone()),
            ..Default::default()
        };

        let config_dir = tmp.join("tmux-test-config");
        std::env::set_var("CLAUDE_CONFIG_DIR", &config_dir);
        let resolved = resolve_project_dir(&options).expect("resolve");
        assert_eq!(resolved, config_dir.join("projects").join(&dashed));

        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }
}
