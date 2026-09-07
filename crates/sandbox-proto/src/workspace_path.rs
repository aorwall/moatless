/// The path a workspace snapshot records its tree under, in every format that
/// stores one.
///
/// Fixed rather than taken from `WORKSPACE_DIR`, so a snapshot's paths do not
/// depend on where the writer had the workspace mounted and a reader needs no
/// knowledge of it. The sandbox passes it as `BackupOptions::as_path` and the
/// backend addresses through it; the two must agree or a read resolves nothing.
pub const SNAPSHOT_ROOT: &str = "/workspace";

/// Whether `path` is a workspace-relative path a
/// [`GetWorkspaceObjectUrlsRequest`] may carry.
///
/// The producer (the sandbox) and the validator (the backend) share this one
/// rule so a path the sandbox sends is never one the backend refuses — a
/// rejected batch would wedge every later mirror sync. S3 keys are opaque, so
/// `..` does not traverse there; the guard exists for the two ends that do
/// resolve it — a restore writes each path under `WORKSPACE_DIR`, and a reader
/// serving a stopped task's files resolves it again. Rejects an absolute path,
/// any `.`/`..` component, an empty path, and one long enough to overrun S3's
/// 1024-byte key limit once the prefix is on it.
pub fn is_safe_workspace_path(path: &str) -> bool {
    if path.is_empty() || path.len() > 900 || path.starts_with('/') {
        return false;
    }
    path.split('/')
        .all(|part| !part.is_empty() && part != "." && part != "..")
}
