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
pub struct LiveProcess {
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

/// Parent PID from `/proc/<pid>/stat` (field 4, after the comm parens).
fn parent_of(pid: u32) -> Option<u32> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = &stat[stat.rfind(')')? + 1..];
    after_comm.split_whitespace().nth(1)?.parse().ok()
}

/// The chain from `leaf` up to (but excluding) `root`.
///
/// In tmux mode this is the persistent session's own spine — the CLI in the
/// pane and the tmux server above it — which is alive for the sandbox's whole
/// life and must not be mistaken for background work.
fn ancestry_to_root(leaf: u32, root: u32) -> Vec<u32> {
    ancestry_with(leaf, root, parent_of)
}

/// Ancestry walk with an injected parent lookup, so the traversal rules are
/// testable without `/proc`.
fn ancestry_with(leaf: u32, root: u32, parent: impl Fn(u32) -> Option<u32>) -> Vec<u32> {
    let mut chain = Vec::new();
    let mut cur = leaf;
    // Bounded: a spine deeper than this means a cycle or a surprise tree, and
    // walking forever inside a status call would be worse than reporting none.
    for _ in 0..32 {
        if cur == root || cur == 0 {
            break;
        }
        chain.push(cur);
        match parent(cur) {
            Some(next) => cur = next,
            None => break,
        }
    }
    chain
}

/// Live descendants of `root`, minus the persistent session spine leading down
/// to `pane_pid`.
///
/// The tmux transport keeps a tmux server and a CLI process alive from session
/// start to sandbox shutdown, so rooting a plain walk at PID 1 reports "busy"
/// forever. Excluding just that spine leaves exactly the agent's own work —
/// both jobs the CLI manages (children of the pane process) and jobs detached
/// with `setsid`/`nohup`, which reparent to PID 1 and sit *outside* the pane's
/// subtree. The walk still descends *through* the excluded PIDs, so the CLI's
/// own children are still found.
pub fn live_descendants_excluding_session(root: u32, pane_pid: u32) -> Vec<LiveProcess> {
    let spine: Vec<u32> = ancestry_to_root(pane_pid, root);
    live_descendants(root)
        .into_iter()
        .filter(|p| !spine.contains(&p.pid))
        .collect()
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
pub fn live_descendants(root: u32) -> Vec<LiveProcess> {
    let mut out = Vec::new();
    let mut stack = children_of(root);
    while let Some(pid) = stack.pop() {
        // Always descend into children — a zombie can still parent live work.
        stack.extend(children_of(pid));
        if pid_is_live(pid) {
            out.push(LiveProcess {
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
    pids.iter()
        .copied()
        .filter(|&pid| pid_is_live(pid))
        .collect()
}

/// Traversal rules, testable on any host via the injected parent lookup.
#[cfg(test)]
mod ancestry_tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(pairs: &[(u32, u32)]) -> impl Fn(u32) -> Option<u32> + '_ {
        let map: HashMap<u32, u32> = pairs.iter().copied().collect();
        move |pid| map.get(&pid).copied()
    }

    /// The measured tmux shape: sandbox(1) → tmux server(60) → claude(61).
    #[test]
    fn spine_is_the_chain_from_the_pane_up_to_the_root() {
        let chain = ancestry_with(61, 1, lookup(&[(61, 60), (60, 1)]));
        assert_eq!(chain, vec![61, 60], "tmux server and CLI are the session");
    }

    #[test]
    fn root_itself_is_never_part_of_the_spine() {
        assert!(ancestry_with(1, 1, lookup(&[])).is_empty());
    }

    #[test]
    fn walk_stops_when_a_parent_is_unreadable() {
        // The process exited mid-walk — take what was resolved, don't spin.
        let chain = ancestry_with(61, 1, lookup(&[(61, 60)]));
        assert_eq!(chain, vec![61, 60]);
    }

    #[test]
    fn a_parent_cycle_cannot_hang_the_status_call() {
        let chain = ancestry_with(5, 1, lookup(&[(5, 6), (6, 5)]));
        assert_eq!(chain.len(), 32, "bounded rather than looping forever");
    }
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

        // `spawn()` returns as soon as the child PID exists — before it has
        // finished `exec`-ing into /bin/sleep. Until the exec lands,
        // /proc/<pid>/cmdline still reflects this test binary's argv, so poll
        // briefly until the descendant surfaces with its own command.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let found = loop {
            if let Some(proc) = live_descendants(std::process::id())
                .into_iter()
                .find(|p| p.pid == child_pid && p.command.contains("sleep"))
            {
                break proc;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "spawned sleep should appear among live descendants with its argv"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        assert!(found.command.contains("sleep"));

        // any_pid_live tracks the specific pid while alive, and flips after exit.
        assert!(any_pid_live(&[child_pid]));

        child.kill().expect("kill sleep");
        child.wait().expect("reap sleep");

        assert!(
            !live_descendants(child_pid)
                .iter()
                .any(|p| p.pid == child_pid),
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
