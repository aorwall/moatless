//! Whether a volume the pod kept already holds the task's tree, which decides
//! if a restore may extract over it.

use std::path::{Path, PathBuf};

use crate::config::Config;

/// Whether the local volume already holds the tree the task works in: a usable
/// primary repo, or, for a task that clones nothing, any entry at all.
///
/// A task with no checkout has no `.git` to test, so without the second arm a
/// kept volume never counts and every restart extracts the snapshot over it.
pub(crate) async fn volume_is_usable(config: &Config) -> bool {
    if config.repos.is_empty() {
        let workspace_dir = PathBuf::from(&config.workspace_dir);
        return tokio::task::spawn_blocking(move || {
            std::fs::read_dir(workspace_dir).is_ok_and(|mut entries| entries.next().is_some())
        })
        .await
        .unwrap_or(false);
    }
    primary_repo_is_usable(Path::new(&config.repo_dir)).await
}

/// Whether the local volume already holds a usable primary repo.
///
/// Deliberately the same test the agent would fail on: a `.git` that exists
/// *and* answers `git rev-parse HEAD`. Anything else — empty, half-extracted,
/// corrupted — reads as "not usable" and lets the snapshot win.
pub(crate) async fn primary_repo_is_usable(repo_dir: &Path) -> bool {
    if !repo_dir.join(".git").exists() {
        return false;
    }
    tokio::process::Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(["rev-parse", "HEAD"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.email=t@t.co", "-c", "user.name=t"])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    #[tokio::test]
    async fn a_committed_repo_is_usable() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        std::fs::write(repo.join("a.txt"), "hello").unwrap();
        git(&repo, &["add", "a.txt"]);
        git(&repo, &["commit", "-q", "-m", "init"]);

        assert!(primary_repo_is_usable(&repo).await);
    }

    #[tokio::test]
    async fn an_empty_or_broken_repo_is_not_usable() {
        let tmp = tempfile::tempdir().unwrap();

        // Nothing there at all — a freshly created PVC.
        let missing = tmp.path().join("missing");
        assert!(!primary_repo_is_usable(&missing).await);

        // A directory with files but no `.git` — a half-extracted tar.
        let no_git = tmp.path().join("no-git");
        std::fs::create_dir_all(&no_git).unwrap();
        std::fs::write(no_git.join("a.txt"), "hello").unwrap();
        assert!(!primary_repo_is_usable(&no_git).await);

        // A `.git` that exists but cannot answer `rev-parse HEAD` — an init'd
        // repo with no commit, or one corrupted by a kill mid-write.
        let no_head = tmp.path().join("no-head");
        std::fs::create_dir_all(&no_head).unwrap();
        git(&no_head, &["init", "-q"]);
        assert!(!primary_repo_is_usable(&no_head).await);
    }

    /// A task with no checkout has no `.git` to probe, so a kept volume counts
    /// as soon as it holds anything — otherwise every restart reverts it.
    #[tokio::test]
    async fn a_volume_with_no_checkout_is_usable_once_it_holds_a_file() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = crate::config::test_support::minimal_config();
        config.workspace_dir = tmp.path().to_string_lossy().into_owned();
        config.repo_dir = config.workspace_dir.clone();
        assert!(!volume_is_usable(&config).await, "a fresh volume is empty");

        std::fs::write(tmp.path().join("notes.md"), "work").unwrap();
        assert!(volume_is_usable(&config).await);
    }
}
