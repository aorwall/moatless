//! What a workspace sync stores in the `restic` repository. See
//! [`collect_repo`] and [`collect_tree`] for what each directory contributes.
//!
//! The tar is **not** on this walk. `sync::append_tree`, `sync::append_repo`
//! and `sync::workspace_fingerprint` each carry their own copy of the same
//! rule; see `docs/deferred-changes.md`.

use std::path::{Path, PathBuf};

/// Every file under `workspace_dir` a sync stores, unsorted and rooted at
/// `workspace_dir`.
///
/// Runs `git` as a subprocess and blocks on `std::fs`, so the caller belongs on
/// a blocking thread.
pub(super) fn collect(workspace_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if workspace_dir.is_dir() {
        collect_tree(&mut files, workspace_dir);
    }
    files
}

/// A git repo root is enumerated through git and not descended into; every
/// other directory is walked.
fn collect_tree(files: &mut Vec<PathBuf>, dir: &Path) {
    if dir.join(".git").is_dir() {
        collect_repo(files, dir);
        return;
    }
    let Ok(dir_entries) = std::fs::read_dir(dir) else {
        return;
    };
    for path in dir_entries.flatten().map(|e| e.path()) {
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            collect_tree(files, &path);
        } else if meta.is_file() {
            files.push(path);
        }
    }
}

/// The working-tree file set from `git ls-files -c -o --exclude-standard` plus
/// the whole `.git`, so ignored build output stays out while HEAD, the index
/// and the dirty tree come back exactly.
fn collect_repo(files: &mut Vec<PathBuf>, repo_dir: &Path) {
    if let Ok(output) = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(["ls-files", "-c", "-o", "--exclude-standard", "-z"])
        .output()
    {
        if output.status.success() {
            for rel in output.stdout.split(|b| *b == 0) {
                if rel.is_empty() {
                    continue;
                }
                let Ok(rel) = std::str::from_utf8(rel) else {
                    continue;
                };
                let full = repo_dir.join(rel);
                let Ok(meta) = std::fs::symlink_metadata(&full) else {
                    // `ls-files` lists paths, not on-disk state, so a
                    // tracked-but-deleted file has nothing to store.
                    continue;
                };
                if meta.is_file() || meta.is_symlink() {
                    files.push(full);
                }
            }
        }
    }
    collect_dir(files, &repo_dir.join(".git"));
}

/// Every regular file under `dir`. Used for a repo's `.git`, which has no
/// gitignore to honor.
fn collect_dir(files: &mut Vec<PathBuf>, dir: &Path) {
    let Ok(dir_entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in dir_entries.flatten() {
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            collect_dir(files, &path);
        } else if meta.is_file() && !is_git_lock(&path) {
            files.push(path);
        }
    }
}

/// Whether `path` is one of the lock files under a repo's `.git`.
///
/// Every caller walks a `.git`, which is what makes matching any `*.lock`
/// right: outside one `Cargo.lock` matches too, and dropping it restores as
/// deleted. Git releases a lock by rename, so a stored one restores with no
/// owner and every `git` there fails with `Unable to create ... .lock`.
pub(super) fn is_git_lock(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "lock")
}

/// Every git lock file under `workspace_dir`, for the restore to delete.
///
/// The inverse of what [`collect`] stores. A lock reaches a fresh pod from a
/// snapshot written before [`is_git_lock`] excluded them, or from a volume the
/// previous pod left it on; either way its owner is gone and it blocks `git`.
pub(super) fn collect_git_locks(workspace_dir: &Path) -> Vec<PathBuf> {
    let mut locks = Vec::new();
    if workspace_dir.is_dir() {
        collect_locks_tree(&mut locks, workspace_dir);
    }
    locks
}

/// A git repo root hands its `.git` to [`collect_locks_in`] and is not
/// descended into; every other directory is walked.
fn collect_locks_tree(locks: &mut Vec<PathBuf>, dir: &Path) {
    if dir.join(".git").is_dir() {
        collect_locks_in(locks, &dir.join(".git"));
        return;
    }
    let Ok(dir_entries) = std::fs::read_dir(dir) else {
        return;
    };
    for path in dir_entries.flatten().map(|e| e.path()) {
        // `symlink_metadata`, so a link back up the tree cannot loop the walk.
        if std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir()) {
            collect_locks_tree(locks, &path);
        }
    }
}

/// Every lock under a `.git`.
fn collect_locks_in(locks: &mut Vec<PathBuf>, dir: &Path) {
    let Ok(dir_entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in dir_entries.flatten() {
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            collect_locks_in(locks, &path);
        } else if meta.is_file() && is_git_lock(&path) {
            locks.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lock_held_during_the_walk_is_not_collected() {
        let tmp = tempfile::tempdir().unwrap();
        let git = tmp.path().join("repo/.git");
        std::fs::create_dir_all(git.join("refs/heads")).unwrap();
        std::fs::write(git.join("HEAD"), b"ref: refs/heads/main\n").unwrap();
        std::fs::write(git.join("index"), b"").unwrap();
        std::fs::write(git.join("index.lock"), b"").unwrap();
        std::fs::write(git.join("refs/heads/main.lock"), b"").unwrap();

        let names: Vec<String> = collect(tmp.path())
            .iter()
            .map(|path| path.display().to_string())
            .collect();

        assert!(names.iter().any(|name| name.ends_with("/.git/HEAD")));
        assert!(names.iter().any(|name| name.ends_with("/.git/index")));
        assert!(
            !names
                .iter()
                .any(|name| name.ends_with("index.lock") || name.ends_with("main.lock")),
            "{names:?}"
        );
    }

    /// The restore deletes what this returns, so a working-tree lockfile — which
    /// satisfies the same predicate — must never reach it.
    #[test]
    fn only_locks_under_git_are_swept() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join(".git/refs/heads")).unwrap();
        std::fs::write(repo.join(".git/index.lock"), b"").unwrap();
        std::fs::write(repo.join(".git/refs/heads/main.lock"), b"").unwrap();
        std::fs::write(repo.join("Cargo.lock"), b"").unwrap();
        std::fs::write(repo.join("bun.lock"), b"").unwrap();

        let mut locks: Vec<String> = collect_git_locks(tmp.path())
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        locks.sort();

        assert_eq!(locks.len(), 2, "{locks:?}");
        assert!(locks[0].ends_with("/.git/index.lock"), "{locks:?}");
        assert!(
            locks[1].ends_with("/.git/refs/heads/main.lock"),
            "{locks:?}"
        );
    }
}
