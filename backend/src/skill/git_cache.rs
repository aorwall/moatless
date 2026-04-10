//! Backend-side shallow sparse clone cache for `.claude/skills/`.
//!
//! Each git URL maps deterministically to a directory under the configured
//! cache root. Clones use `--depth 1 --filter=blob:none --sparse` followed by
//! `git sparse-checkout set .claude/skills` so only the skill tree is
//! materialised on disk. Subsequent fetches reuse the existing clone.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use tokio::process::Command;

use super::error::SkillError;

/// Abstraction over `fetch_or_update` so the orchestrating service can be
/// unit-tested without shelling out to git.
#[async_trait]
pub trait GitFetcher: Send + Sync {
    async fn fetch_or_update(
        &self,
        git_url: &str,
        token: Option<&str>,
    ) -> Result<PathBuf, SkillError>;
}

/// Filesystem-backed [`GitFetcher`] that invokes the `git` CLI.
pub struct FsGitFetcher {
    cache_root: PathBuf,
}

impl FsGitFetcher {
    pub fn new(cache_root: impl Into<PathBuf>) -> Self {
        Self {
            cache_root: cache_root.into(),
        }
    }

    fn dest_for(&self, git_url: &str) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(git_url.as_bytes());
        let hex = hex::encode(hasher.finalize());
        self.cache_root.join(hex)
    }
}

#[async_trait]
impl GitFetcher for FsGitFetcher {
    async fn fetch_or_update(
        &self,
        git_url: &str,
        token: Option<&str>,
    ) -> Result<PathBuf, SkillError> {
        tokio::fs::create_dir_all(&self.cache_root).await?;
        let dest = self.dest_for(git_url);
        let auth_url = token.map_or_else(
            || git_url.to_string(),
            |t| inject_token_into_url(git_url, t),
        );

        if dest.join(".git").exists() {
            update_existing(&dest, git_url).await?;
        } else {
            clone_fresh(&auth_url, &dest, git_url).await?;
        }
        Ok(dest)
    }
}

async fn clone_fresh(auth_url: &str, dest: &Path, original_url: &str) -> Result<(), SkillError> {
    // Clean any partial directory from a previous failed attempt.
    if dest.exists() {
        tokio::fs::remove_dir_all(dest).await?;
    }
    let dest_str = dest.to_string_lossy().to_string();

    run_git(
        &[
            "clone",
            "--depth",
            "1",
            "--filter=blob:none",
            "--sparse",
            "--no-checkout",
            auth_url,
            &dest_str,
        ],
        None,
        original_url,
    )
    .await?;

    run_git(
        &["sparse-checkout", "set", ".claude/skills"],
        Some(dest),
        original_url,
    )
    .await?;

    run_git(&["checkout"], Some(dest), original_url).await?;

    Ok(())
}

async fn update_existing(dest: &Path, original_url: &str) -> Result<(), SkillError> {
    run_git(
        &["fetch", "--depth", "1", "origin", "HEAD"],
        Some(dest),
        original_url,
    )
    .await?;
    run_git(
        &["reset", "--hard", "FETCH_HEAD"],
        Some(dest),
        original_url,
    )
    .await?;
    Ok(())
}

async fn run_git(args: &[&str], cwd: Option<&Path>, git_url: &str) -> Result<(), SkillError> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    // Never prompt for credentials on private repos — fail fast instead.
    cmd.env("GIT_TERMINAL_PROMPT", "0");

    let output = cmd.output().await.map_err(|e| SkillError::Git {
        git_url: git_url.to_string(),
        message: format!("failed to spawn git: {e}"),
    })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if is_auth_error(&stderr) {
        Err(SkillError::AuthRequired {
            git_url: git_url.to_string(),
        })
    } else {
        Err(SkillError::Git {
            git_url: git_url.to_string(),
            message: stderr.trim().to_string(),
        })
    }
}

fn is_auth_error(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("authentication failed")
        || lower.contains("could not read username")
        || lower.contains("could not read password")
        || lower.contains("terminal prompts disabled")
        || lower.contains("invalid username or password")
        || lower.contains("permission denied")
        || lower.contains("403 forbidden")
        || lower.contains("401 unauthorized")
}

/// Inject a token into an HTTPS clone URL.
///
/// Mirrors `sandbox/src/init/git.rs::inject_token_into_url`. Duplicated here
/// to avoid a cross-crate dependency on the sandbox binary.
fn inject_token_into_url(url: &str, token: &str) -> String {
    if let Some(rest) = url.strip_prefix("https://") {
        let host_and_path = if let Some(at_pos) = rest.find('@') {
            &rest[at_pos + 1..]
        } else {
            rest
        };
        format!("https://x-access-token:{token}@{host_and_path}")
    } else {
        url.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inject_token_basic() {
        assert_eq!(
            inject_token_into_url("https://github.com/owner/repo.git", "tok"),
            "https://x-access-token:tok@github.com/owner/repo.git"
        );
    }

    #[test]
    fn inject_token_replaces_existing_credentials() {
        assert_eq!(
            inject_token_into_url("https://old:cred@github.com/owner/repo.git", "new"),
            "https://x-access-token:new@github.com/owner/repo.git"
        );
    }

    #[test]
    fn inject_token_ssh_passthrough() {
        let ssh = "git@github.com:owner/repo.git";
        assert_eq!(inject_token_into_url(ssh, "tok"), ssh);
    }

    #[test]
    fn detects_auth_error_from_git_stderr() {
        assert!(is_auth_error("fatal: Authentication failed for 'https://...'"));
        assert!(is_auth_error("could not read Username for 'https://...': terminal prompts disabled"));
        assert!(is_auth_error("remote: HTTP Basic: Access denied\nfatal: Authentication failed"));
        assert!(!is_auth_error("fatal: repository 'https://...' not found"));
    }

    #[test]
    fn dest_for_is_deterministic_sha256_path() {
        let fetcher = FsGitFetcher::new("/tmp/cache");
        let a = fetcher.dest_for("https://github.com/a/b.git");
        let b = fetcher.dest_for("https://github.com/a/b.git");
        let c = fetcher.dest_for("https://github.com/a/c.git");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.starts_with("/tmp/cache"));
    }
}
