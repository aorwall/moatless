//! Whether a volume the pod kept already holds the task's tree, which decides
//! if a restore may extract over it, and whether a restored checkout is one
//! `git` can read.

use std::path::{Path, PathBuf};

use tracing::{error, warn};

use super::notices::publish_workspace_event;
use crate::config::Config;
use crate::state::SandboxState;

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

/// What [`set_aside_checkout`] did with a restored primary checkout.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum DamagedCheckout {
    /// `git` reads the checkout, an unborn `HEAD` included, or nothing is
    /// there.
    Readable,
    /// Moved to `path`, so the clone gets an empty directory. `git_kept` means
    /// the copy's `.git` could not be renamed, so no snapshot stores its tree.
    SetAside { path: PathBuf, git_kept: bool },
    /// Damaged and left in place: it is the workspace dir itself, or the
    /// rename failed.
    LeftInPlace,
}

/// Move a restored primary checkout that `git` cannot read out of the clone's
/// way, keeping its working tree for the agent to recover from.
///
/// Left in place, a damaged `.git` is still a `.git`, so
/// `init::git::clone_repositories` skips the checkout and the agent starts in
/// a directory where every `git` command fails. Blocks on `std::fs` and `git`.
pub(super) fn set_aside_checkout(workspace_dir: &Path, repo_dir: &Path) -> DamagedCheckout {
    if !repo_dir.exists() || checkout_is_readable(repo_dir) {
        return DamagedCheckout::Readable;
    }
    if repo_dir == workspace_dir || !repo_dir.starts_with(workspace_dir) {
        return DamagedCheckout::LeftInPlace;
    }
    let Some(name) = repo_dir.file_name() else {
        return DamagedCheckout::LeftInPlace;
    };
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let aside = repo_dir.with_file_name(format!("{}.unreadable-{stamp}", name.to_string_lossy()));
    if let Err(error) = std::fs::rename(repo_dir, &aside) {
        warn!(%error, from = %repo_dir.display(), to = %aside.display(), "Could not set the damaged checkout aside");
        return DamagedCheckout::LeftInPlace;
    }
    // A `.git` dir makes `file_set::collect` list the copy through `git
    // ls-files`, which fails on it and stores none of the working tree.
    let git = aside.join(".git");
    let mut git_kept = false;
    if git.exists() {
        if let Err(error) = std::fs::rename(&git, aside.join(".git.unreadable")) {
            warn!(%error, dir = %git.display(), "Could not rename the damaged .git; the next snapshot keeps none of the moved copy's working tree");
            git_kept = true;
        }
    }
    DamagedCheckout::SetAside {
        path: aside,
        git_kept,
    }
}

/// Whether `git status` succeeds in `repo_dir` on its own `.git`.
///
/// Not `rev-parse HEAD`, which also fails on the unborn `HEAD` of a clone of
/// an empty repository. `GIT_CEILING_DIRECTORIES` stops a broken `.git` from
/// sending discovery up to a repository above the checkout.
fn checkout_is_readable(repo_dir: &Path) -> bool {
    if !repo_dir.join(".git").exists() {
        return false;
    }
    let mut command = std::process::Command::new("git");
    command
        .arg("-C")
        .arg(repo_dir)
        .args(["status", "--porcelain"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(parent) = repo_dir.parent() {
        command.env("GIT_CEILING_DIRECTORIES", parent);
    }
    command.status().is_ok_and(|status| status.success())
}

/// Move a restored primary checkout `git` cannot read aside so init reclones
/// it, and say where its working tree went; see
/// [`set_aside_checkout`].
pub(super) async fn set_aside_damaged_checkout(
    state: &SandboxState,
    workspace_dir: PathBuf,
    repo_dir: PathBuf,
) {
    let dir = repo_dir.clone();
    let outcome =
        tokio::task::spawn_blocking(move || set_aside_checkout(&workspace_dir, &repo_dir))
            .await
            .unwrap_or(DamagedCheckout::LeftInPlace);
    let message = match outcome {
        DamagedCheckout::Readable => return,
        DamagedCheckout::SetAside { path, git_kept } => format!(
            "The restored checkout at {} was not one git can read, so it was moved to {} and the repository was cloned again. Uncommitted work from before the restart is in the moved copy.{}",
            dir.display(),
            path.display(),
            if git_kept {
                " Its `.git` could not be renamed, so no later snapshot stores that copy's files: copy out what you need before the sandbox restarts again."
            } else {
                ""
            }
        ),
        DamagedCheckout::LeftInPlace => format!(
            "The restored checkout at {} is not one git can read, and it could not be moved aside, so git commands there fail.",
            dir.display()
        ),
    };
    error!(task_id = %state.config.task_id, dir = %dir.display(), "{message}");
    publish_workspace_event(state, "workspace_checkout_unreadable", &message);
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

    /// The shape a snapshot taken mid-restore leaves: a `.git` holding some of
    /// its files, which the clone would skip as already cloned.
    #[test]
    fn a_checkout_git_cannot_read_is_set_aside_with_its_working_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join(".git/objects")).unwrap();
        std::fs::write(repo.join("edited.rs"), "work").unwrap();

        let outcome = set_aside_checkout(tmp.path(), &repo);

        let DamagedCheckout::SetAside {
            path: aside,
            git_kept: false,
        } = outcome
        else {
            panic!("a damaged checkout was not set aside: {outcome:?}");
        };
        assert!(!repo.exists(), "the clone needs the directory free");
        assert_eq!(
            std::fs::read_to_string(aside.join("edited.rs")).unwrap(),
            "work"
        );
        // The next snapshot must carry the moved working tree, not only the
        // broken `.git` that `git ls-files` fails on.
        let stored = super::super::file_set::collect(tmp.path()).files;
        assert!(
            stored.contains(&aside.join("edited.rs")),
            "the moved copy's working tree is not snapshotted: {stored:?}"
        );
    }

    #[test]
    fn a_working_tree_with_no_git_at_all_is_set_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(repo.join("a.txt"), "hello").unwrap();

        assert!(matches!(
            set_aside_checkout(tmp.path(), &repo),
            DamagedCheckout::SetAside {
                git_kept: false,
                ..
            }
        ));
    }

    /// A clone of an empty repository has no commit, and its work is real.
    #[test]
    fn an_unborn_head_is_readable_and_stays() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        std::fs::write(repo.join("draft.md"), "work").unwrap();

        assert_eq!(
            set_aside_checkout(tmp.path(), &repo),
            DamagedCheckout::Readable
        );
        assert!(repo.join("draft.md").exists());
    }

    /// Renaming the workspace dir would take every sibling with it.
    #[test]
    fn a_damaged_checkout_at_the_workspace_root_is_left_in_place() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".git")).unwrap();

        assert_eq!(
            set_aside_checkout(tmp.path(), tmp.path()),
            DamagedCheckout::LeftInPlace
        );
        assert!(tmp.path().join(".git").is_dir());
    }
}
