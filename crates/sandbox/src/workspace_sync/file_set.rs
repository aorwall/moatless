//! What a workspace sync stores in the `restic` repository.
//!
//! Inside a git repo the set is `git ls-files -c -o --exclude-standard` plus
//! that repo's whole `.git`, so ignored build output stays out while HEAD, the
//! index and the dirty tree come back exactly. Every other directory is walked
//! generically.
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
/// the whole `.git`.
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
        } else if meta.is_file() {
            files.push(path);
        }
    }
}
