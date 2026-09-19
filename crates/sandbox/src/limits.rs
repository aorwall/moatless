//! The ceilings this pod runs inside, which `free` and `df` both miss.
//!
//! Memory is capped by the container's cgroup, far below the node total `free`
//! prints, and the workspace volume by a `sizeLimit` the kubelet meters on its
//! own while `df` reports the node's filesystem. Crossing either ends the work —
//! the cgroup kills the process, and a full volume evicts the pod when it is an
//! `emptyDir` and fails the write when it is a PVC — so both numbers are read
//! here and handed to the agent and its children.

use moatless_workspace_types::workspace::WORKSPACE_LIMIT_ENV;

/// cgroup v2 memory ceiling for this container. v1's `memory.limit_in_bytes` is
/// deliberately not read: every runtime this sandbox ships on is v2, and a
/// missing file already means "no cap we can name".
const MEMORY_MAX_PATH: &str = "/sys/fs/cgroup/memory.max";

/// Env var carrying [`memory_limit_bytes`] to every child process, so a shell
/// script reads the cap without parsing `/sys`. The agent's prompt names it.
pub const MEMORY_LIMIT_ENV: &str = "SANDBOX_MEMORY_LIMIT";

/// The container's memory ceiling in bytes, or `None` when it is uncapped
/// (`max`) or there is no cgroup file to read.
pub fn memory_limit_bytes() -> Option<u64> {
    parse_memory_max(&std::fs::read_to_string(MEMORY_MAX_PATH).ok()?)
}

/// The workspace volume's size limit as the pod builder wrote it (`20Gi`), or
/// `None` in a runtime that caps nothing.
pub fn workspace_limit() -> Option<String> {
    std::env::var(WORKSPACE_LIMIT_ENV)
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
}

/// `max` — the cgroup's word for uncapped — parses to `None`, same as garbage.
fn parse_memory_max(raw: &str) -> Option<u64> {
    raw.trim().parse::<u64>().ok().filter(|bytes| *bytes > 0)
}

/// The `--max-old-space-size` (in MiB) to give Node under `limit_bytes`.
///
/// Node's default heap comes from the host's total RAM, so inside a pod it
/// picks several times the cgroup cap and is killed at the cap rather than
/// collecting under it. Half leaves the rest of the container its room.
pub fn node_max_old_space_mb(limit_bytes: u64) -> u64 {
    (limit_bytes / 2 / (1024 * 1024)).max(512)
}

/// The env pairs carrying this pod's memory cap to every child process, empty
/// where the cgroup names no cap.
///
/// `NODE_OPTIONS` is included only when nothing set it already, so a repo's own
/// value still wins; the caller exports these into the sandbox's own
/// environment, which every spawn inherits.
pub fn memory_env_exports() -> Vec<(&'static str, String)> {
    let Some(bytes) = memory_limit_bytes() else {
        return Vec::new();
    };
    let mut exports = vec![(MEMORY_LIMIT_ENV, bytes.to_string())];
    if std::env::var_os("NODE_OPTIONS").is_none() {
        exports.push((
            "NODE_OPTIONS",
            format!("--max-old-space-size={}", node_max_old_space_mb(bytes)),
        ));
    }
    exports
}

/// Render a byte count as GiB for a human-facing note: `8 GiB`, `1.5 GiB`.
pub fn format_gib(bytes: u64) -> String {
    const GIB: u64 = 1024 * 1024 * 1024;
    let tenths = (bytes.saturating_mul(10)).div_ceil(GIB);
    match tenths % 10 {
        0 => format!("{} GiB", tenths / 10),
        fraction => format!("{}.{fraction} GiB", tenths / 10),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_uncapped_cgroup_names_no_limit() {
        assert_eq!(parse_memory_max("max\n"), None);
        assert_eq!(parse_memory_max(""), None);
        assert_eq!(parse_memory_max("0"), None);
        assert_eq!(parse_memory_max("8589934592\n"), Some(8_589_934_592));
    }

    /// Half the cap, not half the host: a 15 GiB host behind an 8 GiB cgroup is
    /// exactly the case where Node's own default overshoots.
    #[test]
    fn node_gets_half_the_cgroup_cap() {
        assert_eq!(node_max_old_space_mb(8 * 1024 * 1024 * 1024), 4096);
        assert_eq!(node_max_old_space_mb(12 * 1024 * 1024 * 1024), 6144);
        assert_eq!(node_max_old_space_mb(64 * 1024 * 1024), 512);
    }

    #[test]
    fn gib_reads_as_a_person_would_write_it() {
        assert_eq!(format_gib(8 * 1024 * 1024 * 1024), "8 GiB");
        assert_eq!(format_gib(3 * 1024 * 1024 * 1024 / 2), "1.5 GiB");
    }
}
