//! Workspace setup: plugin checkouts and setup commands.

use std::path::Path;

use tokio::fs;
use tokio::process::Command;
use tracing::{info, warn};

use crate::config::Config;

/// Clone or update each of the task owner's plugins under
/// [`Config::plugins_dir`] and remove the skills they did not pick.
///
/// Non-fatal per plugin; see [`checkout_plugin`] for what a restored
/// checkout gets. Prune reruns since picks can change between starts.
pub async fn clone_plugins(config: &Config) -> Vec<String> {
    if config.plugins.is_empty() {
        return Vec::new();
    }

    let base_dir = config.plugins_dir();
    if let Err(e) = fs::create_dir_all(&base_dir).await {
        warn!(error = %e, "Failed to create plugins directory");
        return Vec::new();
    }

    let git_token = std::env::var("GIT_TOKEN").ok();

    let mut cloned_paths = Vec::new();

    for plugin in &config.plugins {
        let name = &plugin.name;
        let dest = base_dir.join(name);

        if let Err(e) = checkout_plugin(&plugin.git_url, &dest, git_token.as_deref()).await {
            warn!(
                name = %name,
                url = %strip_url_credentials(&plugin.git_url),
                error = %e,
                "Failed to clone plugin (non-fatal)"
            );
            continue;
        }

        let dropped = super::plugin_skills::keep_only(&dest, name, &plugin.skill_names);
        if dropped > 0 {
            info!(name = %name, dropped, "Removed skills this task's owner did not pick");
        }
        cloned_paths.push(dest.to_string_lossy().into_owned());
    }

    cloned_paths
}

/// Bring `dest` to the tip of `git_url`'s default branch, shallowly.
///
/// `dest` is snapshotted, so `token` goes only in a per-command header. A
/// restored checkout is fetched, not re-cloned; if the fetch fails it is reset
/// to its own `HEAD`, which restores the skills `keep_only` pruned last start.
async fn checkout_plugin(git_url: &str, dest: &Path, token: Option<&str>) -> Result<(), String> {
    let origin = strip_url_credentials(git_url);
    let auth = token.map(auth_header_config);
    let dest_str = dest.to_string_lossy();

    if dest.join(".git").exists() {
        // Also scrubs a token an older clone embedded in the URL.
        git_in(&dest_str, None, &["remote", "set-url", "origin", &origin]).await?;
        match git_in(
            &dest_str,
            auth.as_deref(),
            &["fetch", "--depth", "1", "origin", "HEAD"],
        )
        .await
        {
            Ok(()) => {
                git_in(&dest_str, None, &["reset", "--hard", "FETCH_HEAD"]).await?;
                info!(dest = %dest_str, "Plugin updated from origin");
            }
            Err(e) => {
                warn!(
                    dest = %dest_str,
                    error = %e,
                    "Failed to fetch plugin; keeping the copy restored from the snapshot"
                );
                git_in(&dest_str, None, &["reset", "--hard", "HEAD"]).await?;
            }
        }
        return Ok(());
    }

    // A directory without `.git` is a clone that died part-way; git refuses to
    // clone into it while it is non-empty.
    if dest.exists() {
        fs::remove_dir_all(dest)
            .await
            .map_err(|e| format!("Failed to remove partial plugin dir: {e}"))?;
    }
    let mut args: Vec<&str> = Vec::new();
    if let Some(auth) = &auth {
        args.extend(["-c", auth]);
    }
    args.extend(["clone", "--depth", "1", &origin, &dest_str]);
    super::git::run_command("git", &args).await?;
    info!(dest = %dest_str, "Plugin cloned successfully");
    Ok(())
}

/// Run `git -C dir [-c auth] args…`.
async fn git_in(dir: &str, auth: Option<&str>, args: &[&str]) -> Result<(), String> {
    let mut full: Vec<&str> = vec!["-C", dir];
    if let Some(auth) = auth {
        full.extend(["-c", auth]);
    }
    full.extend_from_slice(args);
    super::git::run_command("git", &full).await
}

/// The `-c` value that authenticates one git command with `token`, the same
/// `x-access-token` basic credential `inject_token_into_url` puts in a URL.
fn auth_header_config(token: &str) -> String {
    use base64::Engine;
    let basic = base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{token}"));
    format!("http.extraHeader=Authorization: Basic {basic}")
}

/// `url` without any `user:pass@` in front of an HTTPS host.
fn strip_url_credentials(url: &str) -> String {
    let Some(rest) = url.strip_prefix("https://") else {
        return url.to_string();
    };
    let authority_end = rest.find('/').unwrap_or(rest.len());
    match rest[..authority_end].rfind('@') {
        Some(at) => format!("https://{}", &rest[at + 1..]),
        None => url.to_string(),
    }
}

/// Run a single setup command in the repository directory.
pub async fn run_setup_command(config: &Config, cmd: &str) -> Result<(), String> {
    if cmd.is_empty() {
        return Ok(());
    }

    info!(command = %cmd, "Running setup command");
    let output = Command::new("sh")
        .args(["-c", cmd])
        .current_dir(&config.repo_dir)
        .output()
        .await
        .map_err(|e| format!("Failed to execute: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Exit code {}: {stderr}", output.status));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;
    use tempfile::tempdir;

    fn git_sync(dir: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .expect("git command runs");
        assert!(output.status.success(), "git {args:?} failed");
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn commit_file(repo: &Path, rel: &str, contents: &str) {
        let path = repo.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
        git_sync(repo, &["add", "."]);
        git_sync(
            repo,
            &[
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=Test User",
                "commit",
                "-m",
                rel,
            ],
        );
    }

    /// A plugin source repo holding one skill, served over `file://` so the
    /// shallow clone and fetch behave as they do against a remote.
    fn source_repo(root: &Path) -> (PathBuf, String) {
        let source = root.join("source");
        std::fs::create_dir_all(&source).unwrap();
        git_sync(&source, &["init", "--initial-branch", "main"]);
        commit_file(&source, "skills/review/SKILL.md", "v1\n");
        let url = format!("file://{}", source.display());
        (source, url)
    }

    #[tokio::test]
    async fn a_fresh_clone_keeps_the_token_out_of_origin() {
        let root = tempdir().unwrap();
        let (_source, url) = source_repo(root.path());
        let dest = root.path().join("plugins/agentops");

        checkout_plugin(&url, &dest, Some("ghu_secret"))
            .await
            .unwrap();

        assert!(dest.join("skills/review/SKILL.md").is_file());
        let config = std::fs::read_to_string(dest.join(".git/config")).unwrap();
        assert!(!config.contains("ghu_secret"), "token persisted: {config}");
    }

    /// A checkout the snapshot restored moves to the new tip, and a skill the
    /// last start pruned comes back for `keep_only` to judge again.
    #[tokio::test]
    async fn a_restored_checkout_is_fetched_rather_than_recloned() {
        let root = tempdir().unwrap();
        let (source, url) = source_repo(root.path());
        let dest = root.path().join("plugins/agentops");
        checkout_plugin(&url, &dest, None).await.unwrap();
        std::fs::remove_dir_all(dest.join("skills/review")).unwrap();
        commit_file(&source, "skills/lint/SKILL.md", "v2\n");

        checkout_plugin(&url, &dest, None).await.unwrap();

        assert!(dest.join("skills/review/SKILL.md").is_file());
        assert!(dest.join("skills/lint/SKILL.md").is_file());
    }

    /// An unreachable origin keeps the restored copy, and a token an older
    /// clone embedded in `origin` is scrubbed either way.
    #[tokio::test]
    async fn a_failed_fetch_keeps_the_restored_copy() {
        let root = tempdir().unwrap();
        let (_source, url) = source_repo(root.path());
        let dest = root.path().join("plugins/agentops");
        checkout_plugin(&url, &dest, None).await.unwrap();
        std::fs::remove_dir_all(dest.join("skills/review")).unwrap();
        let unreachable = "https://x-access-token:ghu_secret@unreachable.invalid/o/p.git";
        git_sync(&dest, &["remote", "set-url", "origin", unreachable]);

        checkout_plugin(unreachable, &dest, Some("ghu_secret"))
            .await
            .unwrap();

        assert!(dest.join("skills/review/SKILL.md").is_file());
        assert_eq!(
            git_sync(&dest, &["remote", "get-url", "origin"]),
            "https://unreachable.invalid/o/p.git"
        );
    }

    #[tokio::test]
    async fn a_partial_checkout_is_replaced_by_a_clone() {
        let root = tempdir().unwrap();
        let (_source, url) = source_repo(root.path());
        let dest = root.path().join("plugins/agentops");
        std::fs::create_dir_all(dest.join("leftover")).unwrap();

        checkout_plugin(&url, &dest, None).await.unwrap();

        assert!(dest.join("skills/review/SKILL.md").is_file());
        assert!(!dest.join("leftover").exists());
    }

    #[test]
    fn strip_url_credentials_removes_only_userinfo() {
        assert_eq!(
            strip_url_credentials("https://x-access-token:tok@github.com/o/p.git"),
            "https://github.com/o/p.git"
        );
        assert_eq!(
            strip_url_credentials("https://github.com/o/p@v1.git"),
            "https://github.com/o/p@v1.git"
        );
        assert_eq!(
            strip_url_credentials("git@github.com:o/p.git"),
            "git@github.com:o/p.git"
        );
    }
}
