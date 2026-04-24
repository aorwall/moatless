//! Write the GitHub token to `~/.config/gh/hosts.yml` via `gh auth login --with-token`.
//!
//! The `gh` CLI's git credential helper (`gh auth git-credential`) reads this
//! file fresh on every invocation. By writing the current token here on each
//! refresh cycle, git/gh operations spawned from a long-running agent turn pick
//! up rotated tokens without needing the agent's process env to change.

use std::process::Stdio;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tracing::{debug, warn};

/// Write `token` to the gh CLI's hosts.yml via `gh auth login --with-token`.
///
/// `home_dir` is used to locate the resulting `hosts.yml` for permission tightening.
/// Idempotent — safe to call every refresh cycle.
pub async fn write_github_token(home_dir: &str, token: &str) -> Result<(), String> {
    let mut child = Command::new("gh")
        .args(["auth", "login", "--with-token"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn gh auth login: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(token.as_bytes())
            .await
            .map_err(|e| format!("Failed to write token to gh stdin: {e}"))?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(|e| format!("Failed to write newline to gh stdin: {e}"))?;
        drop(stdin);
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| format!("Failed to wait for gh auth login: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh auth login --with-token failed: {stderr}"));
    }

    set_hosts_file_perms(home_dir).await;
    debug!("Refreshed GitHub token in gh hosts.yml");
    Ok(())
}

/// Tighten perms on `~/.config/gh/hosts.yml` to `0600`.
///
/// `gh` writes the file itself, but doesn't guarantee secrets-safe perms.
async fn set_hosts_file_perms(home_dir: &str) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = format!("{home_dir}/.config/gh/hosts.yml");
        if let Err(e) =
            tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).await
        {
            warn!(error = %e, path, "Failed to chmod gh hosts.yml");
        }
    }
    #[cfg(not(unix))]
    {
        let _ = home_dir;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn set_hosts_file_perms_applies_0600() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::TempDir::new().unwrap();
        let gh_dir = tmp.path().join(".config").join("gh");
        tokio::fs::create_dir_all(&gh_dir).await.unwrap();
        let path = gh_dir.join("hosts.yml");
        tokio::fs::write(&path, "github.com:\n  oauth_token: abc\n")
            .await
            .unwrap();
        // Start with world-readable perms so we can confirm we tightened them.
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .await
            .unwrap();

        set_hosts_file_perms(&tmp.path().display().to_string()).await;

        let meta = tokio::fs::metadata(&path).await.unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn set_hosts_file_perms_is_silent_when_missing() {
        let tmp = tempfile::TempDir::new().unwrap();
        // File doesn't exist — helper should log but not panic.
        set_hosts_file_perms(&tmp.path().display().to_string()).await;
    }
}
