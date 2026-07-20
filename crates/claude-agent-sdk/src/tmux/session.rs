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
    /// The SDK-private tmux server socket (`-S <socket>`) this session runs on.
    /// Every tmux command for this session is pinned to it — see
    /// [`tmux_socket_path`] for why we don't use tmux's shared default socket.
    pub(crate) socket: PathBuf,
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
        // Pin claude's config dir + HOME to exactly what the SDK polls, then
        // layer `options.env` after (caller overrides win) — see
        // [`child_env_vars`]. Set on the tmux CLIENT's environment rather than
        // passed as `new-session` argv words: a private-socket session always
        // forks a brand new tmux SERVER (see [`tmux_socket_path`]), which
        // inherits the client's environment at fork time and copies it into
        // the new session, so the pane's spawned process still sees these —
        // without counting them against tmux's `MAX_IMSGSIZE` control-message
        // limit the way argv words would. That limit is what production hit:
        // `options.env` carries every resolved credential/secret (unbounded
        // size), and pushing it through argv turned "tmux new-session" into
        // "command too long".
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
    /// Escalation, mirroring `claude-p`'s sequence:
    /// 1. `tmux kill-session` (graceful — sends SIGHUP via tmux's pane reaper).
    /// 2. Poll `tmux has-session` every 50ms for up to 200ms.
    /// 3. If still alive, grab the pane PID and send SIGKILL directly.
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

    /// Capture the pane PID of the tmux session, used by [`Self::kill`] for
    /// the SIGKILL escalation step. Returns `None` if the session has already
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

/// Path to the SDK's private tmux server socket for `session_name`.
///
/// We run every session on its own `-S <socket>` rather than tmux's shared
/// default socket (`$TMUX_TMPDIR/tmux-<uid>/default`). The default socket is a
/// single well-known path per user: if a previous tmux server died uncleanly
/// and left its socket file behind — e.g. a sandbox container restart, where
/// the memory-backed `/tmp` survives the restart — `new-session` tries to
/// connect to the dead server and fails with "error connecting to <socket>
/// (Connection refused)" (exit 1). A fresh, per-session socket path derived
/// from the unique `session_name` can never collide with such a leftover.
fn tmux_socket_path(session_name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{session_name}.sock"))
}

/// Build a `tmux` command pinned to the SDK's private server socket (`-S`).
///
/// Every tmux invocation for a given session MUST go through this so it targets
/// the same server the session was created on — otherwise `kill-session`,
/// `has-session`, `capture-pane`, and the paste writer would talk to tmux's
/// default socket and never find the session.
pub(super) fn tmux_command(socket: &Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("tmux");
    cmd.arg("-S").arg(socket);
    cmd
}

/// Build the env vars to set on the tmux CLIENT process before `new-session`.
///
/// These end up visible to the pane's `claude` process because a
/// private-socket session (see [`tmux_socket_path`]) always forks a brand new
/// tmux SERVER, which inherits the client's environment at fork time and
/// copies it into the new session's environment table. Setting them via
/// `Command::env` rather than `KEY=VALUE` `new-session` argv words keeps them
/// out of tmux's control-socket message, which is capped at `MAX_IMSGSIZE`
/// (16384 bytes, tmux's `compat/imsg.h`) — a limit `options.env` alone can
/// exceed in production, since it carries every resolved credential/secret
/// (see `sandbox/src/agent/claude_code.rs`), not just the two pins below.
///
/// Meanwhile JSONL discovery ([`resolve_project_dir`]) computes the polled
/// transcript dir from the *SDK process's* env. If the child's
/// `CLAUDE_CONFIG_DIR`/`HOME` don't match, claude writes its transcript
/// somewhere the SDK never looks → `JsonlNotFound`.
///
/// Pinning both here — using the exact resolver discovery uses
/// ([`crate::config_dir::resolve_claude_config_dir`]) — makes "SDK polls X ⟺
/// claude writes X" an invariant by construction. `options.env` is applied
/// after so callers can still override (last write to a given key wins, same
/// as `Command::env` semantics).
fn child_env_vars(options: &ClaudeOptions) -> Vec<(String, String)> {
    let mut vars = Vec::new();

    let home = std::env::var("HOME").unwrap_or_default();
    let config_dir = crate::config_dir::resolve_claude_config_dir(&home);
    vars.push((
        "CLAUDE_CONFIG_DIR".to_string(),
        config_dir.to_string_lossy().into_owned(),
    ));
    if !home.is_empty() {
        vars.push(("HOME".to_string(), home));
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
    let home = std::env::var("HOME").unwrap_or_default();
    let default = format!("{home}/.claude/local/claude");
    if std::path::Path::new(&default).exists() {
        return default;
    }
    "claude".to_string()
}

/// Lexically normalise a raw cwd to match `canonicalize`'s no-trailing-slash
/// form: trim a trailing `/` (preserving root `/`). The dashed encoding in
/// [`resolve_project_dir`] is just `replace('/', "-")`, so matching the
/// trailing-slash shape is all that's needed for the names to line up.
fn lexical_cwd(c: &str) -> String {
    let trimmed = c.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Resolve `<claude-config-dir>/projects/<dashed-cwd>/`.
///
/// The config dir is resolved by the shared [`crate::resolve_claude_config_dir`]
/// helper, which mirrors the Claude CLI's precedence: `CLAUDE_CONFIG_DIR`, then
/// its documented `CLAUDE_HOME` alias, then `$HOME/.claude`. The TUI encodes a
/// cwd by replacing `/` with `-` (e.g. `/Users/x/y` becomes `-Users-x-y`).
///
/// `canonicalize(cwd)` is best-effort: it only serves as the cache-key for the
/// transcript dir, but a transient failure on a known-good absolute path (e.g.
/// an `EACCES` from a SELinux relabel race while a remounted PVC initialises)
/// must not be fatal. On failure we fall back to a lexically-normalised form of
/// the raw cwd and `warn!`. tmux launches claude with the same raw cwd
/// (`tmux new-session ... -c <cwd>`), and Node resolves symlinks at runtime, so
/// for a non-symlink path the lexical fallback yields the identical
/// `projects/<dashed-cwd>` name as canonicalize. In the rare symlink+failure
/// case, discovery falls back to snapshot-diff / the Stop hook.
pub(crate) fn resolve_project_dir(options: &ClaudeOptions) -> Result<PathBuf> {
    let cwd_str = if let Some(c) = &options.cwd {
        match std::path::PathBuf::from(c).canonicalize() {
            Ok(p) => p.to_string_lossy().into_owned(),
            Err(e) => {
                tracing::warn!(
                    cwd = %c,
                    error = %e,
                    "canonicalize(cwd) failed; falling back to lexical path for project-dir resolution"
                );
                lexical_cwd(c)
            }
        }
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

/// Resolve the transcript the SDK should target on disk directly, ahead of
/// snapshot-diff and the Stop hook. Two resume shapes append to an EXISTING
/// transcript (so snapshot-diff never sees a new file):
///
/// - `--resume <id>` → `<id>.jsonl`.
/// - `--continue`    → the most-recently-modified existing transcript, i.e. the
///   one `claude --continue` reopens. Without targeting it directly, discovery
///   falls back to the Stop hook, which only fires at turn-end and so times out
///   with `JsonlNotFound` on long first turns.
///
/// The fork case (`--resume <id> --fork-session`) writes a NEW transcript, so
/// snapshot-diff / Stop handle it — return None there. Returns None when no
/// matching transcript exists (e.g. `--continue` with an empty project dir),
/// leaving discovery to its snapshot-diff / Stop fallbacks.
pub(crate) async fn resolve_resume_target(
    options: &ClaudeOptions,
    project_dir: &Path,
) -> Option<PathBuf> {
    if options.fork_session == Some(true) {
        return None;
    }
    if let Some(id) = options.resume.as_ref() {
        return Some(project_dir.join(format!("{id}.jsonl")));
    }
    if options.continue_conversation == Some(true) {
        return newest_transcript(project_dir).await;
    }
    None
}

/// The most-recently-modified `*.jsonl` in `dir`, or None if there are none.
async fn newest_transcript(dir: &Path) -> Option<PathBuf> {
    let mut read = tokio::fs::read_dir(dir).await.ok()?;
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    while let Ok(Some(entry)) = read.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(mtime) = entry.metadata().await.and_then(|m| m.modified()) else {
            continue;
        };
        if newest.as_ref().is_none_or(|(t, _)| mtime > *t) {
            newest = Some((mtime, path));
        }
    }
    newest.map(|(_, p)| p)
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

/// True if `dir` contains at least one `*.jsonl` transcript. NotFound → false
/// (mirrors [`list_jsonl_stems`]).
pub(crate) async fn has_transcript_in(dir: &Path) -> bool {
    let Ok(mut read) = tokio::fs::read_dir(dir).await else {
        return false;
    };
    while let Ok(Some(entry)) = read.next_entry().await {
        if entry.path().extension().and_then(|s| s.to_str()) == Some("jsonl") {
            return true;
        }
    }
    false
}

/// True when the Claude project dir for `cwd` contains a transcript —
/// i.e. `--continue` has something to reopen. Resolves the dir the same way
/// the tmux transport does (`CLAUDE_CONFIG_DIR` → `CLAUDE_HOME` →
/// `$HOME/.claude`, then `projects/<dashed-cwd>`). A cwd canonicalize failure
/// no longer propagates from [`resolve_project_dir`] (it falls back to the
/// lexical path), so the probe returns `Ok(false)` (start fresh) rather than
/// `Err`; the soft caller handles `Ok(false)` correctly. The `Result` is kept
/// for the remaining I/O-error surface.
pub async fn has_resumable_transcript(cwd: Option<&str>) -> Result<bool> {
    let options = ClaudeOptions {
        cwd: cwd.map(ToString::to_string),
        ..Default::default()
    };
    let project_dir = resolve_project_dir(&options)?;
    Ok(has_transcript_in(&project_dir).await)
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

    // When `canonicalize(cwd)` fails (here `ENOENT` on a nonexistent path,
    // exercising the same fallback branch as the `EACCES` SELinux-relabel race),
    // resolution must NOT error — it falls back to the lexically-normalised raw
    // cwd so transcript discovery still targets `projects/<dashed-cwd>`.
    #[test]
    fn project_dir_falls_back_when_cwd_not_canonicalizable() {
        let _guard = crate::config_dir::ENV_LOCK.lock().expect("env lock");
        let options = ClaudeOptions {
            cwd: Some("/nonexistent/abc".into()),
            ..Default::default()
        };

        let config_dir = std::env::temp_dir().join("tmux-test-config-fallback");
        std::env::set_var("CLAUDE_CONFIG_DIR", &config_dir);
        let resolved = resolve_project_dir(&options).expect("resolve falls back, not errors");
        assert_eq!(
            resolved,
            config_dir.join("projects").join("-nonexistent-abc")
        );

        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[tokio::test]
    async fn has_transcript_in_empty_dir_is_false() {
        let tmp = tempfile::tempdir().expect("create tempdir");
        assert!(!has_transcript_in(tmp.path()).await);
    }

    #[tokio::test]
    async fn has_transcript_in_dir_with_jsonl_is_true() {
        let tmp = tempfile::tempdir().expect("create tempdir");
        // Non-jsonl files must not count as transcripts.
        std::fs::write(tmp.path().join("notes.txt"), "x").expect("write notes.txt");
        assert!(!has_transcript_in(tmp.path()).await);
        std::fs::write(tmp.path().join("x.jsonl"), "{}").expect("write x.jsonl");
        assert!(has_transcript_in(tmp.path()).await);
    }

    #[tokio::test]
    async fn has_transcript_in_missing_dir_is_false() {
        let tmp = tempfile::tempdir().expect("create tempdir");
        assert!(!has_transcript_in(&tmp.path().join("does-not-exist")).await);
    }

    // Asserts the probe resolves the same `projects/<dashed-cwd>` dir under
    // `CLAUDE_CONFIG_DIR` that the tmux transport polls. Mutates process-global
    // env, so it runs under the shared env lock — a sync test with `block_on`
    // rather than `#[tokio::test]`, to avoid holding the guard across awaits.
    #[test]
    fn has_resumable_transcript_resolves_env_config_dir() {
        let _guard = crate::config_dir::ENV_LOCK.lock().expect("env lock");
        // Use an existing directory so canonicalize() succeeds — tempdir works.
        let cwd = std::env::temp_dir();
        let canonical = cwd.canonicalize().expect("temp_dir should canonicalize");
        let dashed = canonical.to_string_lossy().replace('/', "-");

        let config_dir = tempfile::tempdir().expect("create tempdir");
        std::env::set_var("CLAUDE_CONFIG_DIR", config_dir.path());

        let project_dir = config_dir.path().join("projects").join(&dashed);
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        let transcript = project_dir.join("s.jsonl");
        std::fs::write(&transcript, "{}").expect("write transcript");

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build runtime");
        let cwd_str = cwd.to_string_lossy().into_owned();
        let probe = |cwd: &str| rt.block_on(has_resumable_transcript(Some(cwd)));
        assert!(probe(&cwd_str).expect("probe"));

        std::fs::remove_file(&transcript).expect("remove transcript");
        assert!(!probe(&cwd_str).expect("probe"));

        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    // The tmux child env vars must pin `CLAUDE_CONFIG_DIR` (resolved by the
    // SAME helper discovery uses) and `HOME`, so claude writes its transcript to
    // the dir the SDK polls instead of relying on tmux-server env inheritance.
    #[test]
    fn child_env_vars_pins_config_dir_and_home() {
        let _guard = crate::config_dir::ENV_LOCK.lock().expect("env lock");
        let saved_home = std::env::var("HOME").ok();

        std::env::set_var("HOME", "/home/tester");
        std::env::set_var("CLAUDE_CONFIG_DIR", "/opt/moatless/.claude");

        // No options.env: vars are exactly the two pinned assignments.
        let vars = child_env_vars(&ClaudeOptions::default());
        assert!(
            vars.contains(&(
                "CLAUDE_CONFIG_DIR".to_string(),
                "/opt/moatless/.claude".to_string()
            )),
            "vars should pin CLAUDE_CONFIG_DIR: {vars:?}"
        );
        assert!(
            vars.contains(&("HOME".to_string(), "/home/tester".to_string())),
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
            ..Default::default()
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

        std::env::remove_var("CLAUDE_CONFIG_DIR");
        match saved_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
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

    #[tokio::test]
    async fn resume_target_resume_id_points_at_id_jsonl() {
        let dir = std::env::temp_dir();
        let options = ClaudeOptions {
            resume: Some("abc-123".to_string()),
            ..Default::default()
        };
        let target = resolve_resume_target(&options, &dir).await;
        assert_eq!(target, Some(dir.join("abc-123.jsonl")));
    }

    #[tokio::test]
    async fn resume_target_fork_is_none() {
        let dir = std::env::temp_dir();
        let options = ClaudeOptions {
            resume: Some("abc-123".to_string()),
            fork_session: Some(true),
            ..Default::default()
        };
        assert_eq!(resolve_resume_target(&options, &dir).await, None);
    }

    #[tokio::test]
    async fn resume_target_plain_query_is_none() {
        let dir = std::env::temp_dir();
        assert_eq!(
            resolve_resume_target(&ClaudeOptions::default(), &dir).await,
            None
        );
    }

    /// Create a unique empty temp subdir (no `tempfile` dev-dep in this crate).
    fn unique_tmp_dir() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sdk-resume-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[tokio::test]
    async fn resume_target_continue_picks_newest_existing_transcript() {
        let dir = unique_tmp_dir();
        // Two transcripts with explicit, distinct mtimes; --continue must pick
        // the newest (the one claude reopens).
        let older = dir.join("older.jsonl");
        let newer = dir.join("newer.jsonl");
        std::fs::write(&older, b"{}\n").expect("write older");
        std::fs::write(&newer, b"{}\n").expect("write newer");
        let base = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        std::fs::File::open(&older)
            .unwrap()
            .set_modified(base)
            .expect("set older mtime");
        std::fs::File::open(&newer)
            .unwrap()
            .set_modified(base + std::time::Duration::from_secs(60))
            .expect("set newer mtime");
        // A non-jsonl file must be ignored.
        std::fs::write(dir.join("notes.txt"), b"x").expect("write txt");

        let options = ClaudeOptions {
            continue_conversation: Some(true),
            ..Default::default()
        };
        assert_eq!(resolve_resume_target(&options, &dir).await, Some(newer));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn resume_target_continue_empty_dir_is_none() {
        let dir = unique_tmp_dir();
        let options = ClaudeOptions {
            continue_conversation: Some(true),
            ..Default::default()
        };
        assert_eq!(resolve_resume_target(&options, &dir).await, None);
        std::fs::remove_dir_all(&dir).ok();
    }

    // Regression test for the production "command too long" failure (SP tmux
    // investigation, task e0b3e4e8): `options.env` in the real pipeline
    // carries every resolved credential/secret (sandbox/src/agent/claude_code.rs),
    // unbounded in size — before the fix, `child_env_prefix` re-broadcast that
    // whole map as individual `new-session` argv words, and tmux's client
    // packs the entire `new-session` argv into a single control-socket
    // message capped at `MAX_IMSGSIZE` (16384 bytes, compat/imsg.h), failing
    // with exit status 1 and stderr "command too long" once prompt + secrets
    // crossed that ceiling. `child_env_vars` now sets these via `Command::env`
    // on the tmux CLIENT instead (picked up by the freshly-forked tmux SERVER
    // and copied into the session), which isn't bounded by that limit — so a
    // ~4.5KB system prompt (realistic size, matches the failing task's logged
    // `append_system_prompt_len`) plus a handful of secret-sized env values
    // must now succeed.
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

        // Stand-in for a handful of real-world secrets (kubeconfig,
        // service-account JSON, cloud credentials) that `resolve_task_credentials`
        // would merge into `options.env` alongside GitHub/Codex tokens — nothing
        // filters these by size before they reach the sandbox. Combined with
        // the ~4.5KB prompt below, this alone would have exceeded tmux's 16KB
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
            system_prompt: Some(crate::types::options::SystemPrompt::Preset {
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
            system_prompt: Some(crate::types::options::SystemPrompt::Preset {
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

        let dir = unique_tmp_dir();
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
