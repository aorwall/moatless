/// The path a `restic` snapshot records its tree under.
///
/// Fixed rather than taken from `WORKSPACE_DIR`, so a snapshot's paths do not
/// depend on where the writer had the workspace mounted and a reader needs no
/// knowledge of it. The sandbox passes it as `BackupOptions::as_path` and the
/// backend addresses through it; the two must agree or a read resolves nothing.
pub const SNAPSHOT_ROOT: &str = "/workspace";
