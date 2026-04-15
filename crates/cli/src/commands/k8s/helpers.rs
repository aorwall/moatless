//! Shared kubectl / helm subprocess helpers.

use std::process::Stdio;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::client::ClientError;

pub(super) const DEFAULT_RELEASE: &str = "moatless-vibe";
pub(super) const DEFAULT_NAMESPACE: &str = "vibe";
pub(super) const SECRET_NAME: &str = "vibe-secrets";
pub(super) const PULL_SECRET_NAME: &str = "ghcr-secret";
pub(super) const DEFAULT_CHART_PATH: &str = "k8s/helm/moatless-vibe";
pub(super) const DEFAULT_VALUES_FILE: &str = "values.local.yaml";

/// Ensure a required binary is on PATH, or return a helpful `ClientError`.
pub(super) fn require_binary(name: &'static str) -> Result<(), ClientError> {
    which::which(name).map(|_| ()).map_err(|_| {
        ClientError::Other(format!(
            "`{name}` not found on PATH. Install it and re-run (https://kubernetes.io/docs/tasks/tools/ for kubectl, https://helm.sh/docs/intro/install/ for helm)."
        ))
    })
}

/// Output of a completed subprocess run.
pub(super) struct CmdOutput {
    pub(super) status: i32,
    pub(super) stdout: String,
    pub(super) stderr: String,
}

impl CmdOutput {
    pub(super) fn success(&self) -> bool {
        self.status == 0
    }

    /// Return `stdout` on success; map non-zero to a `ClientError` that includes stderr.
    pub(super) fn into_stdout(self, context: &str) -> Result<String, ClientError> {
        if self.success() {
            Ok(self.stdout)
        } else {
            Err(ClientError::Other(format!(
                "{context} failed (exit {}): {}",
                self.status,
                self.stderr.trim()
            )))
        }
    }
}

/// Run `kubectl` with the given args and return captured output.
pub(super) async fn kubectl(args: &[&str]) -> Result<CmdOutput, ClientError> {
    run("kubectl", args, None).await
}

/// Run `helm` with the given args and return captured output.
pub(super) async fn helm(args: &[&str]) -> Result<CmdOutput, ClientError> {
    run("helm", args, None).await
}

/// Run `kubectl` with stdin (used for `kubectl apply -f -`).
pub(super) async fn kubectl_stdin(args: &[&str], stdin: &str) -> Result<CmdOutput, ClientError> {
    run("kubectl", args, Some(stdin)).await
}

/// Stream a child process to inherited stdout/stderr. Useful for long-running
/// `helm upgrade --install --wait` where the user benefits from live output.
pub(super) async fn run_streaming(cmd: &str, args: &[&str]) -> Result<(), ClientError> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| ClientError::Other(format!("Failed to spawn {cmd}: {e}")))?;

    let status = child
        .wait()
        .await
        .map_err(|e| ClientError::Other(format!("Failed to await {cmd}: {e}")))?;

    if !status.success() {
        return Err(ClientError::Other(format!(
            "{cmd} exited with status {}",
            status.code().unwrap_or(-1)
        )));
    }
    Ok(())
}

async fn run(cmd: &str, args: &[&str], stdin: Option<&str>) -> Result<CmdOutput, ClientError> {
    let mut command = Command::new(cmd);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if stdin.is_some() {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }

    let mut child = command
        .spawn()
        .map_err(|e| ClientError::Other(format!("Failed to spawn {cmd}: {e}")))?;

    if let Some(input) = stdin {
        if let Some(mut h) = child.stdin.take() {
            h.write_all(input.as_bytes()).await.map_err(|e| {
                ClientError::Other(format!("Failed to write stdin to {cmd}: {e}"))
            })?;
            h.shutdown().await.map_err(|e| {
                ClientError::Other(format!("Failed to close stdin for {cmd}: {e}"))
            })?;
        }
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| ClientError::Other(format!("Failed to collect {cmd} output: {e}")))?;

    Ok(CmdOutput {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Fetch a JSON document from kubectl. Returns `None` if the resource doesn't exist.
pub(super) async fn kubectl_json(args: &[&str]) -> Result<Option<serde_json::Value>, ClientError> {
    let out = kubectl(args).await?;
    if !out.success() {
        if out.stderr.contains("NotFound")
            || out.stderr.contains("not found")
            || out.stderr.contains("No resources found")
        {
            return Ok(None);
        }
        return Err(ClientError::Other(format!(
            "kubectl {} failed: {}",
            args.join(" "),
            out.stderr.trim()
        )));
    }
    let trimmed = out.stdout.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    serde_json::from_str(trimmed)
        .map(Some)
        .map_err(|e| ClientError::Other(format!("Invalid JSON from kubectl: {e}")))
}

/// Return the current kubectl context name, or None if no config / no current-context.
pub(super) async fn current_context() -> Result<Option<String>, ClientError> {
    let out = kubectl(&["config", "current-context"]).await?;
    if !out.success() {
        return Ok(None);
    }
    let ctx = out.stdout.trim().to_string();
    if ctx.is_empty() {
        Ok(None)
    } else {
        Ok(Some(ctx))
    }
}
