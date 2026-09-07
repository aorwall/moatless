//! What a workspace sync stores, for the mirror and the `restic` repository.
//!
//! One rule for those two, because a format that walked differently would
//! restore a different tree. Inside a git repo the set is
//! `git ls-files -c -o --exclude-standard` plus that repo's whole `.git`, so
//! ignored build output stays out while HEAD, the index and the dirty tree come
//! back exactly. Every other directory is walked generically.
//!
//! The tar is **not** on this walk. `sync::append_tree`, `sync::append_repo`
//! and `sync::workspace_fingerprint` each carry their own copy of the same
//! rule; see `docs/deferred-changes.md`.

use std::path::{Path, PathBuf};

use super::sync::{join_prefix, rel_from};

/// One file the sync stores.
pub(super) struct WorkspaceFile {
    /// Workspace-relative, `/`-separated.
    pub rel: String,
    pub full: PathBuf,
    pub meta: std::fs::Metadata,
}

/// Every file under `workspace_dir` a sync stores, unsorted.
///
/// Runs `git` as a subprocess and blocks on `std::fs`, so the caller belongs on
/// a blocking thread.
pub(super) fn collect(workspace_dir: &Path) -> Vec<WorkspaceFile> {
    let mut files = Vec::new();
    if workspace_dir.is_dir() {
        collect_tree(&mut files, workspace_dir, workspace_dir);
    }
    files
}

/// A git repo root is enumerated through git and not descended into; every
/// other directory is walked.
fn collect_tree(files: &mut Vec<WorkspaceFile>, root: &Path, dir: &Path) {
    if dir.join(".git").is_dir() {
        collect_repo(files, dir, &rel_from(root, dir));
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
            collect_tree(files, root, &path);
        } else if meta.is_file() {
            push(files, rel_from(root, &path), path, meta);
        }
    }
}

/// The working-tree file set from `git ls-files -c -o --exclude-standard` plus
/// the whole `.git`.
fn collect_repo(files: &mut Vec<WorkspaceFile>, repo_dir: &Path, prefix: &str) {
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
                    push(files, join_prefix(prefix, rel), full, meta);
                }
            }
        }
    }
    collect_dir(files, &join_prefix(prefix, ".git"), &repo_dir.join(".git"));
}

/// Every regular file under `dir`, keyed at `<name_prefix>/<path-below-dir>`.
/// Used for a repo's `.git`, which has no gitignore to honor.
fn collect_dir(files: &mut Vec<WorkspaceFile>, name_prefix: &str, dir: &Path) {
    let Ok(dir_entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in dir_entries.flatten() {
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let name = format!("{name_prefix}/{file_name}");
        if meta.is_dir() {
            collect_dir(files, &name, &path);
        } else if meta.is_file() {
            push(files, name, path, meta);
        }
    }
}

fn push(files: &mut Vec<WorkspaceFile>, rel: String, full: PathBuf, meta: std::fs::Metadata) {
    files.push(WorkspaceFile { rel, full, meta });
}
