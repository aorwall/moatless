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

/// A live descendant process of the sandbox, surfaced for the control-panel UI
/// and captured by the background-completion watcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundProcess {
    pub pid: u32,
    /// Full command line (argv joined by spaces), falling back to the `comm`
    /// name when `/proc/<pid>/cmdline` is empty (e.g. kernel threads).
    pub command: String,
}

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

/// Whether `pid` is currently a live (non-zombie, non-dead) process. Zombies
/// (`Z`) and dead (`X`/`x`) entries linger in the table but represent finished
/// work; a missing `/proc` entry (`None`) means the process is already gone.
fn pid_is_live(pid: u32) -> bool {
    !matches!(process_state(pid), Some('Z' | 'X' | 'x') | None)
}

/// Best-effort human-readable command for `pid`. Prefers the full argv from
/// `/proc/<pid>/cmdline` (NUL-separated), falling back to the short `comm`
/// name. `None` when neither is readable (the process exited between reads).
fn process_command(pid: u32) -> Option<String> {
    if let Ok(raw) = fs::read(format!("/proc/{pid}/cmdline")) {
        let joined = raw
            .split(|b| *b == 0)
            .filter(|seg| !seg.is_empty())
            .map(|seg| String::from_utf8_lossy(seg))
            .collect::<Vec<_>>()
            .join(" ");
        let trimmed = joined.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    let comm = fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    let comm = comm.trim();
    if comm.is_empty() {
        None
    } else {
        Some(comm.to_string())
    }
}

/// Whether `root` has any live descendant process. Zombies (`Z`) and dead
/// (`X`/`x`) entries don't count — un-reaped orphans linger in the table but
/// represent finished work.
pub fn has_live_descendants(root: u32) -> bool {
    let mut stack = children_of(root);
    while let Some(pid) = stack.pop() {
        if pid_is_live(pid) {
            return true;
        }
        stack.extend(children_of(pid));
    }
    false
}

/// Every live descendant of `root`, with a best-effort command string. Used to
/// surface background jobs in the control-panel UI and to snapshot the PIDs the
/// background-completion watcher waits on. Order is unspecified.
pub fn live_descendants(root: u32) -> Vec<BackgroundProcess> {
    let mut out = Vec::new();
    let mut stack = children_of(root);
    while let Some(pid) = stack.pop() {
        // Always descend into children — a zombie can still parent live work.
        stack.extend(children_of(pid));
        if pid_is_live(pid) {
            out.push(BackgroundProcess {
                pid,
                command: process_command(pid).unwrap_or_else(|| "unknown".to_string()),
            });
        }
    }
    out
}

/// Whether any PID in `pids` is still a live process. Backs the watcher's
/// completion check — it waits on the specific processes captured at arm time
/// rather than "any descendant", so a later agent turn's CLI subprocess never
/// keeps the watch alive.
pub fn any_pid_live(pids: &[u32]) -> bool {
    pids.iter().copied().any(pid_is_live)
}

/// The subset of `pids` that are still live. Used to drop the just-exited CLI
/// subprocess from a freshly-captured watch set after a short settle window,
/// leaving only genuine background work.
pub fn live_pids(pids: &[u32]) -> Vec<u32> {
    pids.iter().copied().filter(|&pid| pid_is_live(pid)).collect()
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

    #[test]
    fn live_descendants_lists_child_with_command() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let child_pid = child.id();

        let procs = live_descendants(std::process::id());
        let found = procs
            .iter()
            .find(|p| p.pid == child_pid)
            .expect("spawned sleep should appear among live descendants");
        assert!(
            found.command.contains("sleep"),
            "command should reflect the process argv, got {:?}",
            found.command
        );

        // any_pid_live tracks the specific pid while alive, and flips after exit.
        assert!(any_pid_live(&[child_pid]));

        child.kill().expect("kill sleep");
        child.wait().expect("reap sleep");

        assert!(
            !live_descendants(child_pid).iter().any(|p| p.pid == child_pid),
            "a reaped leaf has no live descendants of its own"
        );
        assert!(!any_pid_live(&[child_pid]));
    }

    #[test]
    fn any_pid_live_is_false_for_missing_pid() {
        // PID 0 never has a /proc entry.
        assert!(!any_pid_live(&[0]));
        assert!(!any_pid_live(&[]));
    }

    #[test]
    fn live_pids_keeps_only_running_processes() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let pid = child.id();

        // A live child is retained; a bogus PID 0 is filtered out.
        let live = live_pids(&[pid, 0]);
        assert_eq!(live, vec![pid]);

        child.kill().expect("kill sleep");
        child.wait().expect("reap sleep");

        assert!(live_pids(&[pid]).is_empty());
    }
}
