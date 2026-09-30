/// The path a `restic` snapshot records its tree under.
///
/// Fixed rather than tied to `WORKSPACE_DIR`, so paths don't depend on mount
/// location; sandbox and backend must agree on it via `BackupOptions::as_path`
/// or a read resolves nothing.
pub const SNAPSHOT_ROOT: &str = "/workspace";

/// Where `rustic_core` keeps its local pack cache, in the sandbox and the
/// backend alike.
///
/// Mandatory: without a cache `rustic_core` issues one range GET per blob.
/// `/tmp` rather than `WORKSPACE_DIR` (the cache would snapshot itself),
/// `/opt/moatless/data` (capped) or `/run/moatless` (1Mi).
pub const RESTIC_CACHE_DIR: &str = "/tmp/rustic-cache";

/// Directories a workspace file tree never lists, whether walked live in the
/// sandbox or read from a snapshot: unfiltered, a listing would be mostly
/// loose objects, past every caller's cap.
pub const UNBROWSABLE_DIRS: &[&str] = &[".git", "node_modules"];
