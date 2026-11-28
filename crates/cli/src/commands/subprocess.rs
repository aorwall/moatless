//! Subprocess runner shared by the `docker` and `k8s` installers.

use std::process::Stdio;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::client::ClientError;

/// Output of a completed subprocess run.
pub(crate) struct CmdOutput {
    pub(crate) status: i32,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

impl CmdOutput {
    pub(crate) fn success(&self) -> bool {
        self.status == 0
    }
}

pub(crate) async fn run(
    cmd: &str,
    args: &[&str],
    stdin: Option<&str>,
) -> Result<CmdOutput, ClientError> {
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

    if let Some(input) = stdin
        && let Some(mut h) = child.stdin.take()
    {
        h.write_all(input.as_bytes())
            .await
            .map_err(|e| ClientError::Other(format!("Failed to write stdin to {cmd}: {e}")))?;
        h.shutdown()
            .await
            .map_err(|e| ClientError::Other(format!("Failed to close stdin for {cmd}: {e}")))?;
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

/// Run `program` with stdout and stderr inherited, for long commands whose
/// live output the user watches; `label` names it in errors.
pub(crate) async fn run_streaming(
    program: &str,
    label: &str,
    args: &[&str],
) -> Result<(), ClientError> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| ClientError::Other(format!("Failed to spawn {label}: {e}")))?;

    let status = child
        .wait()
        .await
        .map_err(|e| ClientError::Other(format!("Failed to await {label}: {e}")))?;

    if !status.success() {
        return Err(ClientError::Other(format!(
            "{label} exited with status {}",
            status.code().unwrap_or(-1)
        )));
    }
    Ok(())
}
