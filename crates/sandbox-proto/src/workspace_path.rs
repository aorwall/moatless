/// The path a `restic` snapshot records its tree under.
///
/// Fixed rather than tied to `WORKSPACE_DIR`, so paths don't depend on mount
/// location; sandbox and backend must agree on it via `BackupOptions::as_path`
/// or a read resolves nothing.
pub const SNAPSHOT_ROOT: &str = "/workspace";
