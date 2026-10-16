//! What a workspace sync stores in the `restic` repository. See
//! [`collect_repo`] and [`collect_tree`] for what each directory contributes.
//!
//! The tar is **not** on this walk. `sync::append_tree`, `sync::append_repo`
//! and `sync::workspace_fingerprint` each carry their own copy of the same
//! rule; see `docs/deferred-changes.md`.

use std::path::{Path, PathBuf};

pub(super) use crate::init::git::clone_fetch_incomplete;

/// What a sync stores under a workspace, and the directories its walk may skip.
#[derive(Default)]
pub(super) struct FileSet {
    /// Every file a sync stores, unsorted and rooted at the workspace dir.
    pub(super) files: Vec<PathBuf>,
    /// Directories git ignores whole, such as `target/` and `node_modules/`.
    /// May still hold a stored file; `restic::write_glob_file` checks.
    pub(super) ignored_dirs: Vec<PathBuf>,
}

/// The [`FileSet`] of `workspace_dir`.
///
/// Runs `git` as a subprocess and blocks on `std::fs`, so the caller belongs on
/// a blocking thread.
pub(super) fn collect(workspace_dir: &Path) -> FileSet {
    let mut set = FileSet::default();
    if workspace_dir.is_dir() {
        collect_tree(&mut set, workspace_dir);
    }
    set
}

/// A git repo root is enumerated through git and not descended into; every
/// other directory is walked. A clone whose fetch never finished is left out,
/// so init reclones it ([`clone_fetch_incomplete`]).
fn collect_tree(set: &mut FileSet, dir: &Path) {
    if clone_fetch_incomplete(dir) {
        return;
    }
    if is_repo_root(dir) {
        collect_repo(set, dir);
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
            collect_tree(set, &path);
        } else if meta.is_file() {
            set.files.push(path);
        }
    }
}

/// The working-tree file set from `git ls-files -c -o --exclude-standard` plus
/// the whole `.git`, so ignored build output stays out while HEAD, the index
/// and the dirty tree come back exactly.
fn collect_repo(set: &mut FileSet, repo_dir: &Path) {
    for rel in git_ls_files(repo_dir, &["-c", "-o", "--exclude-standard"]) {
        let full = repo_dir.join(rel);
        let Ok(meta) = std::fs::symlink_metadata(&full) else {
            // `ls-files` lists paths, not on-disk state, so a
            // tracked-but-deleted file has nothing to store.
            continue;
        };
        if meta.is_file() || meta.is_symlink() {
            set.files.push(full);
        }
    }
    // `--directory` names a wholly ignored directory once, with a trailing `/`,
    // rather than listing what is inside it.
    for rel in git_ls_files(repo_dir, &["-o", "-i", "--exclude-standard", "--directory"]) {
        if let Some(dir) = rel.strip_suffix('/') {
            set.ignored_dirs.push(repo_dir.join(dir));
        }
    }
    let git = repo_dir.join(".git");
    if git.is_file() {
        set.files.push(git);
    } else {
        collect_dir(&mut set.files, &git);
    }
}

/// The paths `git ls-files <args>` prints, relative to `repo_dir`; none when git
/// fails.
fn git_ls_files(repo_dir: &Path, args: &[&str]) -> Vec<String> {
    let Ok(output) = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .arg("ls-files")
        .args(args)
        .arg("-z")
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    output
        .stdout
        .split(|b| *b == 0)
        .filter(|rel| !rel.is_empty())
        .filter_map(|rel| std::str::from_utf8(rel).ok())
        .map(str::to_string)
        .collect()
}

/// Whether `dir` is a git repo root: its `.git` is a directory, or a linked
/// worktree's `.git` file whose `gitdir:` target exists.
pub(super) fn is_repo_root(dir: &Path) -> bool {
    let git = dir.join(".git");
    if git.is_dir() {
        return true;
    }
    let Ok(contents) = std::fs::read_to_string(&git) else {
        return false;
    };
    contents
        .lines()
        .find_map(|line| line.strip_prefix("gitdir:"))
        .is_some_and(|gitdir| dir.join(gitdir.trim()).is_dir())
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
    if is_repo_root(dir) {
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
            .files
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

    fn git(dir: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.email=t@t.co", "-c", "user.name=t"])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}: {output:?}");
    }

    /// `repo/` with `target/` ignored and one commit, plus a linked worktree
    /// of it at `wt/`.
    fn repo_with_worktree(root: &Path) -> PathBuf {
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        std::fs::write(repo.join(".gitignore"), b"target/\n").unwrap();
        std::fs::write(repo.join("main.rs"), b"fn main() {}\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        git(&repo, &["worktree", "add", "-q", "--detach", "../wt"]);
        let wt = root.join("wt");
        std::fs::create_dir_all(wt.join("target")).unwrap();
        std::fs::write(wt.join("target/big.bin"), b"build output").unwrap();
        std::fs::write(wt.join("src.rs"), b"untracked").unwrap();
        wt
    }

    fn names_under(root: &Path, files: &[PathBuf]) -> Vec<String> {
        files
            .iter()
            .map(|path| path.strip_prefix(root).unwrap().display().to_string())
            .collect()
    }

    #[test]
    fn a_worktree_beside_its_repo_honors_its_gitignore() {
        let tmp = tempfile::tempdir().unwrap();
        repo_with_worktree(tmp.path());

        let names = names_under(tmp.path(), &collect(tmp.path()).files);

        for stored in ["wt/.git", "wt/main.rs", "wt/src.rs", "repo/main.rs"] {
            assert!(
                names.iter().any(|name| name == stored),
                "{stored}: {names:?}"
            );
        }
        assert!(
            names
                .iter()
                .any(|name| name.starts_with("repo/.git/worktrees/wt/")),
            "{names:?}"
        );
        assert!(
            !names.iter().any(|name| name.starts_with("wt/target")),
            "{names:?}"
        );
    }

    #[test]
    fn a_wholly_ignored_directory_is_named_once() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = repo_with_worktree(tmp.path());
        std::fs::create_dir_all(wt.join("target/debug/deps")).unwrap();
        std::fs::write(wt.join("target/debug/deps/a.rlib"), b"build output").unwrap();

        let set = collect(tmp.path());

        assert_eq!(names_under(tmp.path(), &set.ignored_dirs), ["wt/target"]);
    }

    /// `git ls-files` fails without the gitdir, so the worktree is kept whole
    /// rather than stored as nothing.
    #[test]
    fn a_worktree_whose_gitdir_is_gone_is_walked_whole() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = tmp.path().join("wt");
        std::fs::create_dir_all(wt.join("target")).unwrap();
        std::fs::write(wt.join(".git"), b"gitdir: /nonexistent/.git/worktrees/wt\n").unwrap();
        std::fs::write(wt.join(".gitignore"), b"target/\n").unwrap();
        std::fs::write(wt.join("target/big.bin"), b"build output").unwrap();
        std::fs::write(wt.join("main.rs"), b"fn main() {}\n").unwrap();

        let mut names = names_under(tmp.path(), &collect(tmp.path()).files);
        names.sort();

        assert_eq!(
            names,
            [
                "wt/.git",
                "wt/.gitignore",
                "wt/main.rs",
                "wt/target/big.bin"
            ],
        );
    }
}
