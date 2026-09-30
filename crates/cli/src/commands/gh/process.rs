//! Spawning the real `gh`, and the exit status its failure travels on.

use super::token::github_token_for_gh;
use crate::config::Config;

/// A subprocess status returned to the executable entry point.
#[derive(Debug, thiserror::Error)]
#[error("gh exited with status {0}")]
pub struct CommandExit(pub i32);

pub(super) fn command_status(code: i32) -> Result<(), Box<dyn std::error::Error>> {
    if code == 0 {
        Ok(())
    } else {
        Err(Box::new(CommandExit(code)))
    }
}

/// Exec the local `gh` binary with the original args, inheriting stdio, and
/// propagate its exit code.
pub(super) fn passthrough(
    cfg: &Config,
    args: &[String],
    gh_repo: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Thread the git-credential token exactly as the typed and stack paths do,
    // so a passthrough (`pr view`, `api`, `run watch`, …) authenticates the
    // same way. See [`super::token::resolve_gh_token`] for which source wins.
    let token = github_token_for_gh(cfg.github_token_set);
    let status = gh_command(args, token.as_deref(), gh_repo)
        .status()
        .map_err(|e| format!("failed to run gh: {e} (is the gh CLI installed?)"))?;
    command_status(status.code().unwrap_or(1))
}

pub(super) fn gh_command(
    args: &[String],
    token: Option<&str>,
    gh_repo: Option<&str>,
) -> std::process::Command {
    let mut cmd = std::process::Command::new("gh");
    cmd.args(args);
    if let Some(token) = token {
        cmd.env("GH_TOKEN", token);
    }
    // An explicit `-R` on the command line still wins: gh reads the flag first.
    if let Some(gh_repo) = gh_repo {
        cmd.env("GH_REPO", gh_repo);
    }
    cmd
}

pub(crate) fn gh_command_output(
    args: &[String],
    token: Option<&str>,
    gh_repo: Option<&str>,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let output = gh_command(args, token, gh_repo)
        .output()
        .map_err(|e| format!("failed to run gh: {e} (is the gh CLI installed?)"))?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    Err(format!(
        "`gh {}` failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    )
    .into())
}

#[cfg(test)]
mod tests;
