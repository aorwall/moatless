//! Snapshot existing `*.jsonl` files before spawning via `tmux new-session -d`,
//! then hand off to the tailer. The TUI writes no JSONL until the first user
//! message is submitted — paste a prompt before calling [`discover_new_jsonl`]
//! to wait for the file.
//!
//! Cleanup runs through `kill()` (supervisor, on cancellation) and a
//! best-effort sync kill in `Drop` as a safety net.

use std::path::{Path, PathBuf};

use tracing::{debug, info, warn};

use crate::api::options::ClaudeOptions;
use crate::error::{Result, SdkError};

use super::args::build_tmux_cli_args;

/// Default tmux window dimensions for the detached session.
const TMUX_WIDTH: u32 = 220;
const TMUX_HEIGHT: u32 = 50;

/// One spawned tmux + claude session.
///
/// JSONL discovery is intentionally NOT part of this struct — the TUI defers
/// the transcript until the first user message, so `query.rs` must paste a
/// prompt before discovering it.
pub(crate) struct TmuxSession {
    pub(crate) session_name: String,
    /// The SDK-private tmux server socket (`-S <socket>`) this session runs on.
    /// Every tmux command for this session is pinned to it — see
    /// [`tmux_socket_path`] for why we don't use tmux's shared default socket.
    pub(crate) socket: PathBuf,
}

impl TmuxSession {
    /// Spawn a detached tmux session running `claude` with the given options.
    /// Does not wait for the JSONL transcript — see [`discover_new_jsonl`].
    ///
    /// `settings_json` is the harness-owned inline `--settings` JSON, or `None`
    /// to skip the flag entirely.
    pub(crate) async fn spawn(
        options: &ClaudeOptions,
        settings_json: Option<&str>,
    ) -> Result<Self> {
        let cli_path = resolve_cli_path(options);

        let session_name = format!(
            "claude-sdk-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        );
        let socket = tmux_socket_path(&session_name);
        let args = build_tmux_cli_args(options, settings_json);
        info!(
            session_name = %session_name,
            socket = %socket.display(),
            cli_path = %cli_path,
            args = ?args,
            cwd = ?options.cwd,
            "Spawning tmux session for Claude TUI"
        );

        let mut cmd = tmux_command(&socket);
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
        // Pin claude's config dir + HOME to what the SDK polls, then layer
        // `options.env` after (caller wins) — see [`child_env_vars`]. Set on
        // the tmux CLIENT's env, not `new-session` argv: argv is capped by
        // tmux's `MAX_IMSGSIZE`, and `options.env` carries every resolved
        // credential unbounded — argv overflow fails "command too long".
        for (k, v) in child_env_vars(options) {
            cmd.env(k, v);
        }
        cmd.arg("--");
        // `env -u CLAUDECODE` mirrors transport/subprocess.rs:46 — claude
        // refuses to start when it sees CLAUDECODE set.
        cmd.arg("env").arg("-u").arg("CLAUDECODE");
        cmd.arg(&cli_path);
        for a in &args {
            cmd.arg(a);
        }

        // Capture stderr (rather than `.status()` with inherited stdio) so a
        // failed `new-session` surfaces tmux's actual complaint — e.g.
        // "error connecting to <socket> (Connection refused)" or a permission
        // error — instead of an opaque exit code.
        let output = cmd.output().await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                SdkError::TmuxNotFound
            } else {
                SdkError::SpawnError(e)
            }
        })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr = stderr.trim();
            let detail = if stderr.is_empty() {
                String::new()
            } else {
                format!(": {stderr}")
            };
            return Err(SdkError::Internal(format!(
                "tmux new-session exited with status {:?}{detail}",
                output.status
            )));
        }

        debug!(session_name = %session_name, "tmux session spawned");
        Ok(Self {
            session_name,
            socket,
        })
    }

    /// Kill the tmux session (and the claude process inside).
    ///
    /// Escalates: graceful `tmux kill-session` (SIGHUP), poll `tmux
    /// has-session` for up to 200ms, then SIGKILL the pane PID directly if
    /// still alive.
    pub(crate) async fn kill(&self) {
        debug!(session_name = %self.session_name, "Killing tmux session");

        // Capture the pane PID BEFORE killing the session — once tmux exits,
        // `display-message` returns nothing.
        let pane_pid = self.pane_pid().await;

        let result = tmux_command(&self.socket)
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

    /// Capture the pane PID of the tmux session, used by [`Self::kill`] for the
    /// SIGKILL escalation step. Returns `None` if the session has already
    /// exited or the PID couldn't be parsed.
    pub(crate) async fn pane_pid(&self) -> Option<u32> {
        let out = tmux_command(&self.socket)
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

    /// Capture the current pane contents, CSI-stripped. Returns `None` if the
    /// session is already gone or the tmux command fails.
    pub(crate) async fn capture_pane_stripped(&self) -> Option<Vec<u8>> {
        let out = tmux_command(&self.socket)
            .args(["capture-pane", "-t", &self.session_name, "-p"])
            .stderr(std::process::Stdio::null())
            .output()
            .await
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(super::trust::strip_csi(&out.stdout))
    }

    /// Return true if the tmux session is still alive.
    pub(crate) async fn is_alive(&self) -> bool {
        tmux_command(&self.socket)
            .args(["has-session", "-t", &self.session_name])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .is_ok_and(|s| s.success())
    }
}

impl Drop for TmuxSession {
    fn drop(&mut self) {
        // Best-effort synchronous kill in case the async cleanup path didn't
        // run (e.g. supervisor task panicked). Won't block long.
        let _ = std::process::Command::new("tmux")
            .arg("-S")
            .arg(&self.socket)
            .args(["kill-session", "-t", &self.session_name])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        // The tmux server unlinks its socket when it exits, but remove it
        // best-effort in case the server was already gone — leaves no stale
        // socket behind on the (memory-backed) tmp filesystem.
        let _ = std::fs::remove_file(&self.socket);
    }
}

/// Each session gets its own `-S <socket>` instead of tmux's shared default: a
/// leftover socket from an uncleanly-died server (e.g. surviving a container
/// restart) makes `new-session` fail connecting to the dead server, which a
/// name-derived per-session path can never collide with.
fn tmux_socket_path(session_name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{session_name}.sock"))
}

/// Build a `tmux` command pinned to the SDK's private server socket (`-S`).
///
/// Every tmux invocation for a session MUST go through this to target the
/// server it was created on, or `kill-session`/`has-session`/`capture-pane`/the
/// paste writer hit the default socket and never find it.
pub(super) fn tmux_command(socket: &Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("tmux");
    cmd.arg("-S").arg(socket);
    cmd
}

/// Build the env vars to set on the tmux CLIENT process before `new-session`.
///
/// The tmux server passes them to the pane's `claude`. They go through
/// `Command::env` because `new-session` argv caps at tmux's 16384-byte
/// `MAX_IMSGSIZE`. `CLAUDE_CONFIG_DIR` matches JSONL discovery's
/// [`crate::config_dir::claude_config_dir`]; `options.env` applies after.
fn child_env_vars(options: &ClaudeOptions) -> Vec<(String, String)> {
    let mut vars = Vec::new();

    let config_dir = crate::config_dir::claude_config_dir(options);
    vars.push((
        "CLAUDE_CONFIG_DIR".to_string(),
        config_dir.to_string_lossy().into_owned(),
    ));
    let home = crate::config_dir::home_dir();
    if !home.as_os_str().is_empty() {
        vars.push(("HOME".to_string(), home.to_string_lossy().into_owned()));
    }

    if let Some(env) = &options.env {
        for (k, v) in env {
            vars.push((k.clone(), v.clone()));
        }
    }

    vars
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
    let default = crate::config_dir::home_dir().join(".claude/local/claude");
    if default.exists() {
        return default.to_string_lossy().into_owned();
    }
    "claude".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    // The tmux child env vars must pin `CLAUDE_CONFIG_DIR` (resolved by the
    // SAME helper discovery uses) and `HOME`, so claude writes its transcript to
    // the dir the SDK polls instead of relying on tmux-server env inheritance.
    #[test]
    fn child_env_vars_pins_config_dir_and_home() {
        let pinned_options = || ClaudeOptions {
            config_dir: Some("/opt/moatless/.claude".to_string()),
            ..Default::default()
        };

        // No options.env: vars are exactly the two pinned assignments.
        let vars = child_env_vars(&pinned_options());
        assert!(
            vars.contains(&(
                "CLAUDE_CONFIG_DIR".to_string(),
                "/opt/moatless/.claude".to_string()
            )),
            "vars should pin CLAUDE_CONFIG_DIR: {vars:?}"
        );
        let home = crate::config_dir::home_dir().to_string_lossy().into_owned();
        assert!(
            home.is_empty() || vars.contains(&("HOME".to_string(), home.clone())),
            "vars should pin HOME: {vars:?}"
        );

        // options.env is applied AFTER the pins so caller overrides win (last
        // write to a given key wins, matching `Command::env` semantics) —
        // verify both entries are present and the override comes after the pin.
        let mut env = std::collections::HashMap::new();
        env.insert(
            "CLAUDE_CONFIG_DIR".to_string(),
            "/caller/override".to_string(),
        );
        let options = ClaudeOptions {
            env: Some(env),
            ..pinned_options()
        };
        let vars = child_env_vars(&options);
        let pinned = vars
            .iter()
            .position(|(k, v)| k == "CLAUDE_CONFIG_DIR" && v == "/opt/moatless/.claude")
            .expect("pinned config dir present");
        let override_pos = vars
            .iter()
            .position(|(k, v)| k == "CLAUDE_CONFIG_DIR" && v == "/caller/override")
            .expect("caller override present");
        assert!(
            override_pos > pinned,
            "caller override must come after the pin (Command::env: later wins): {vars:?}"
        );
    }

    // Works whether or not tmux is installed: a missing binary and a missing
    // session both surface as `None`.
    #[tokio::test]
    async fn capture_pane_stripped_nonexistent_session_is_none() {
        let session = TmuxSession {
            session_name: "claude-sdk-test-no-such-session".to_string(),
            socket: tmux_socket_path("claude-sdk-test-no-such-session"),
        };
        assert_eq!(session.capture_pane_stripped().await, None);
        // Prevent Drop from running `tmux kill-session` against a live server.
        std::mem::forget(session);
    }

    #[test]
    fn socket_path_is_private_and_per_session() {
        // Socket lives under the process temp dir (known-writable — the same
        // place the hook FIFO is created) and is keyed on the unique session
        // name, so it can never collide with tmux's shared default socket or a
        // stale leftover from a prior server.
        let a = tmux_socket_path("claude-sdk-aaaaaaaa");
        let b = tmux_socket_path("claude-sdk-bbbbbbbb");
        assert_ne!(a, b);
        assert!(a.starts_with(std::env::temp_dir()));
        assert_eq!(a.file_name().unwrap(), "claude-sdk-aaaaaaaa.sock");
    }

    // Regression: `options.env` carries every resolved secret, unbounded, and
    // tmux's control-socket argv is capped at `MAX_IMSGSIZE` (16384 bytes) —
    // `child_env_vars` sets these via `Command::env` on the tmux client
    // instead of `new-session` argv, so prompt + secrets no longer hit
    // "command too long".
    fn realistic_append_system_prompt() -> String {
        "x".repeat(4500)
    }

    #[tokio::test]
    async fn new_session_succeeds_when_env_secrets_are_large() {
        if which_tmux().is_none() {
            eprintln!("tmux not on PATH — skipping");
            return;
        }

        let harness = crate::tmux::hooks::HookHarness::create().expect("create hook harness");

        // Stand-in for real-world secrets (kubeconfig, service-account JSON,
        // cloud credentials) `resolve_task_credentials` merges into
        // `options.env` — nothing filters these by size, and combined with
        // the ~4.5KB prompt below this alone exceeded tmux's 16KB
        // MAX_IMSGSIZE before the fix.
        let mut env = std::collections::HashMap::new();
        env.insert("KUBECONFIG_BLOB".to_string(), "k".repeat(5000));
        env.insert("SERVICE_ACCOUNT_JSON".to_string(), "s".repeat(5000));
        env.insert("CLOUD_CREDENTIALS".to_string(), "c".repeat(5000));

        let options = ClaudeOptions {
            cli_path: Some("/bin/echo".to_string()),
            cwd: Some(std::env::temp_dir().to_string_lossy().into_owned()),
            model: Some("claude-opus-4-8".to_string()),
            disallowed_tools: Some(vec!["AskUserQuestion".to_string()]),
            system_prompt: Some(crate::api::options::SystemPrompt::Preset {
                prompt_type: "preset".to_string(),
                preset: "claude_code".to_string(),
                append: Some(realistic_append_system_prompt()),
            }),
            env: Some(env),
            ..Default::default()
        };

        let session = TmuxSession::spawn(&options, Some(&harness.settings_json))
            .await
            .expect("large env/secrets must no longer overflow tmux's MAX_IMSGSIZE");
        session.kill().await;
    }

    #[tokio::test]
    async fn new_session_succeeds_with_realistic_prompt_and_no_secrets() {
        if which_tmux().is_none() {
            eprintln!("tmux not on PATH — skipping");
            return;
        }

        let harness = crate::tmux::hooks::HookHarness::create().expect("create hook harness");

        let options = ClaudeOptions {
            cli_path: Some("/bin/echo".to_string()),
            cwd: Some(std::env::temp_dir().to_string_lossy().into_owned()),
            model: Some("claude-opus-4-8".to_string()),
            disallowed_tools: Some(vec!["AskUserQuestion".to_string()]),
            system_prompt: Some(crate::api::options::SystemPrompt::Preset {
                prompt_type: "preset".to_string(),
                preset: "claude_code".to_string(),
                append: Some(realistic_append_system_prompt()),
            }),
            ..Default::default()
        };

        let session = TmuxSession::spawn(&options, Some(&harness.settings_json))
            .await
            .expect("prompt alone (no large env) must stay well under tmux's MAX_IMSGSIZE");
        session.kill().await;
    }

    // The fix's core claim: env vars set via `Command::env` on the tmux
    // CLIENT (see `child_env_vars`) still reach the pane's spawned process,
    // even though they no longer travel through `new-session` argv. If this
    // were false, the fix would silently drop credentials instead of loudly
    // failing with "command too long" — worse than the original bug.
    #[tokio::test]
    async fn child_env_vars_reach_the_spawned_pane_process() {
        if which_tmux().is_none() {
            eprintln!("tmux not on PATH — skipping");
            return;
        }

        let dir = std::env::temp_dir().join(format!("claude-sdk-session-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create session directory");
        let dump_path = dir.join("env-dump.txt");
        let script_path = dir.join("dump-env.sh");
        std::fs::write(
            &script_path,
            format!(
                "#!/bin/sh\necho \"$MOAT_TEST_ENV_PROBE\" > {}\n",
                dump_path.display()
            ),
        )
        .expect("write probe script");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o700))
            .expect("chmod probe script");

        let mut env = std::collections::HashMap::new();
        env.insert(
            "MOAT_TEST_ENV_PROBE".to_string(),
            "reached-the-pane".to_string(),
        );
        let options = ClaudeOptions {
            cli_path: Some(script_path.to_string_lossy().into_owned()),
            env: Some(env),
            ..Default::default()
        };

        let session = TmuxSession::spawn(&options, None)
            .await
            .expect("spawn probe session");

        let mut seen = String::new();
        for _ in 0..40 {
            if let Ok(contents) = std::fs::read_to_string(&dump_path) {
                if !contents.trim().is_empty() {
                    seen = contents.trim().to_string();
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        session.kill().await;
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(
            seen, "reached-the-pane",
            "MOAT_TEST_ENV_PROBE set via Command::env on the tmux client must reach the pane"
        );
    }

    fn which_tmux() -> Option<String> {
        std::process::Command::new("which")
            .arg("tmux")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    }
}
