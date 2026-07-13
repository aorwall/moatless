//! Process-tree inspection for the busy-process signal.
//!
//! Walks `/proc/<pid>/task/<tid>/children` recursively to detect live
//! (non-zombie) descendants of the sandbox process. Background jobs spawned
//! by the agent CLI (`run_in_background` builds, ad-hoc daemons) outlive the
//! per-turn CLI subprocess and get reparented to the sandbox process
//! (container PID 1), so any live descendant after the agent goes idle means
//! work is still running. Non-Linux hosts have no `/proc` — every read fails
//! soft and the walk reports "not busy".

use std::fs;

/// Direct children of `pid` across all its threads, from
/// `/proc/<pid>/task/<tid>/children`. Empty when the process is gone or the
/// kernel lacks `CONFIG_PROC_CHILDREN`.
fn children_of(pid: u32) -> Vec<u32> {
    let entries = match fs::read_dir(format!("/proc/{pid}/task")) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        if let Ok(contents) = fs::read_to_string(entry.path().join("children")) {
            out.extend(
                contents
                    .split_whitespace()
                    .filter_map(|s| s.parse::<u32>().ok()),
            );
        }
    }
    out
}

/// Process state letter from `/proc/<pid>/stat`, `None` if unreadable (the
/// process exited between reads). The comm field can contain spaces and
/// parens, so the state is parsed from after the *last* `)`.
fn process_state(pid: u32) -> Option<char> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = &stat[stat.rfind(')')? + 1..];
    after_comm.split_whitespace().next()?.chars().next()
}

/// Whether `root` has any live descendant process. Zombies (`Z`) and dead
/// (`X`/`x`) entries don't count — un-reaped orphans linger in the table but
/// represent finished work.
pub fn has_live_descendants(root: u32) -> bool {
    let mut stack = children_of(root);
    while let Some(pid) = stack.pop() {
        match process_state(pid) {
            Some('Z' | 'X' | 'x') | None => {}
            Some(_) => return true,
        }
        stack.extend(children_of(pid));
    }
    false
}

// /proc is Linux-only; dev machines (macOS) skip these.
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn own_state_is_live() {
        let state = process_state(std::process::id()).expect("own /proc stat is readable");
        assert_ne!(state, 'Z');
    }

    #[test]
    fn missing_process_reads_soft() {
        // PID 0 has no /proc entry.
        assert_eq!(process_state(0), None);
        assert!(children_of(0).is_empty());
        assert!(!has_live_descendants(0));
    }

    #[test]
    fn detects_live_child_and_ignores_leaf() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");

        // The test process now has a live descendant...
        assert!(has_live_descendants(std::process::id()));
        // ...while the leaf `sleep` itself has none.
        assert!(!has_live_descendants(child.id()));

        child.kill().expect("kill sleep");
        child.wait().expect("reap sleep");
    }
}
