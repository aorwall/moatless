//! Ownership of the per-task host directories a sandbox bind-mounts.

use std::io;
use std::os::unix::fs::{MetadataExt, lchown};
use std::path::Path;

use super::container_builder::SANDBOX_UID;

/// Give `dir` and everything under it to the sandbox user, walking the tree
/// only when `dir` itself is owned by someone else (Kubernetes'
/// `fsGroupChangePolicy: OnRootMismatch`).
///
/// The backend creates these directories as root, and a task started before
/// sandboxes ran non-root holds root-owned files its agent could not write.
pub fn chown_to_sandbox_user(dir: &Path) -> io::Result<()> {
    chown_on_mismatch(dir, SANDBOX_UID, SANDBOX_UID)
}

fn chown_on_mismatch(dir: &Path, uid: u32, gid: u32) -> io::Result<()> {
    if std::fs::symlink_metadata(dir)?.uid() == uid {
        return Ok(());
    }
    chown_tree(dir, uid, gid)
}

/// `lchown`, so a symlink the agent left pointing outside the task directory
/// changes owner itself rather than its target.
fn chown_tree(path: &Path, uid: u32, gid: u32) -> io::Result<()> {
    lchown(path, Some(uid), Some(gid))?;
    if std::fs::symlink_metadata(path)?.is_dir() {
        for entry in std::fs::read_dir(path)? {
            chown_tree(&entry?.path(), uid, gid)?;
        }
    }
    Ok(())
}

/// Group owning the Docker socket at `path`, so a non-root sandbox given the
/// socket can use it; `None` when the backend cannot see the socket there.
pub fn socket_gid(path: &str) -> Option<u32> {
    std::fs::metadata(path).ok().map(|meta| meta.gid())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chowning to the caller's own ids needs no privilege, so the walk runs
    /// as any user; a dangling symlink proves it never follows links.
    #[test]
    fn walk_reaches_nested_entries_without_following_symlinks() {
        let dir = tempfile::tempdir().expect("temp dir");
        let nested = dir.path().join("a/b");
        std::fs::create_dir_all(&nested).expect("mkdir");
        std::fs::write(nested.join("f"), b"x").expect("write");
        std::os::unix::fs::symlink("/nonexistent/target", nested.join("link")).expect("symlink");
        let meta = std::fs::metadata(dir.path()).expect("stat");

        chown_tree(dir.path(), meta.uid(), meta.gid()).expect("walk");
    }

    #[test]
    fn owned_dir_is_not_walked() {
        let dir = tempfile::tempdir().expect("temp dir");
        let uid = std::fs::metadata(dir.path()).expect("stat").uid();
        // A gid no non-root user may chown to: reaching `lchown` would fail.
        chown_on_mismatch(dir.path(), uid, 0).expect("short-circuits on owner match");
    }

    #[test]
    fn socket_gid_is_none_for_a_missing_socket() {
        assert_eq!(socket_gid("/nonexistent/docker.sock"), None);
    }
}
