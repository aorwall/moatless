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
use std::time::Duration;

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
    pub(crate) async fn spawn(options: &ClaudeOptions) -> Result<Self> {
        let cli_path = resolve_cli_path(options);

        let session_name = format!(
            "claude-sdk-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        );
        let args = build_tmux_cli_args(options);
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
    pub(crate) async fn kill(&self) {
        debug!(session_name = %self.session_name, "Killing tmux session");
        let result = tokio::process::Command::new("tmux")
            .args(["kill-session", "-t", &self.session_name])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
        if let Err(e) = result {
            warn!(error = %e, session_name = %self.session_name, "tmux kill-session failed");
        }
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

/// Resolve `~/.claude/projects/<dashed-cwd>/`.
///
/// The TUI encodes a cwd by replacing `/` with `-` (e.g. `/Users/x/y`
/// becomes `-Users-x-y`).
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

    let home = std::env::var("HOME").unwrap_or_default();
    let claude_home = std::env::var("CLAUDE_HOME")
        .map_or_else(|_| PathBuf::from(home).join(".claude"), PathBuf::from);

    Ok(claude_home.join("projects").join(dashed))
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

pub(crate) async fn discover_new_jsonl(
    dir: &Path,
    before: &HashSet<String>,
    timeout: Duration,
) -> Result<PathBuf> {
    let deadline = tokio::time::Instant::now() + timeout;
    let poll_interval = Duration::from_millis(200);

    loop {
        let current = list_jsonl_stems(dir).await?;
        let new_stems: Vec<String> = current.difference(before).cloned().collect();

        if !new_stems.is_empty() {
            if new_stems.len() > 1 {
                warn!(
                    count = new_stems.len(),
                    "Multiple new JSONL files appeared concurrently — picking newest by mtime"
                );
            }
            // Pick the newest by mtime
            let mut best: Option<(PathBuf, std::time::SystemTime)> = None;
            for stem in new_stems {
                let path = dir.join(format!("{stem}.jsonl"));
                if let Ok(meta) = tokio::fs::metadata(&path).await {
                    if let Ok(mtime) = meta.modified() {
                        match &best {
                            Some((_, t)) if *t > mtime => {}
                            _ => best = Some((path, mtime)),
                        }
                    }
                }
            }
            if let Some((path, _)) = best {
                return Ok(path);
            }
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(SdkError::JsonlNotFound {
                project_dir: dir.display().to_string(),
                timeout_ms: timeout.as_millis() as u64,
            });
        }
        tokio::time::sleep(poll_interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_dir_uses_dashed_cwd() {
        // Set CLAUDE_HOME to a predictable value to make the test deterministic.
        let claude_home = std::env::temp_dir().join("tmux-test-home");
        std::env::set_var("CLAUDE_HOME", &claude_home);

        // Use an existing directory so canonicalize() succeeds — tempdir works.
        let tmp = std::env::temp_dir();
        let canonical = tmp.canonicalize().expect("temp_dir should canonicalize");
        let cwd_str = canonical.to_string_lossy().into_owned();

        let options = ClaudeOptions {
            cwd: Some(cwd_str.clone()),
            ..Default::default()
        };
        let resolved = resolve_project_dir(&options).expect("resolve");
        let dashed = cwd_str.replace('/', "-");
        let expected = claude_home.join("projects").join(dashed);
        assert_eq!(resolved, expected);

        std::env::remove_var("CLAUDE_HOME");
    }
}
