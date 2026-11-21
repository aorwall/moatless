//! Live processes in this pod's PID namespace.
//!
//! The sandbox container holds one process tree, and everything in it below
//! this process is something this pod started. Reading the namespace is what
//! covers the writers no registry lists and no group names: an agent tool's own
//! child, a process orphaned by a command that already exited, and one that
//! left its group through `setsid` or `nohup`.

/// A live process this pod did not start itself and cannot account for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignProcess {
    pub pid: u32,
    pub comm: String,
}

/// Every live process apart from this one and its ancestors.
///
/// `None` when `/proc` cannot be read, which denies the final-tree tag: a pod
/// that cannot observe its writers must not pass as quiet. Zombies are skipped;
/// nothing reaps them here, and they write nothing.
pub fn foreign_live_processes() -> Option<Vec<ForeignProcess>> {
    let mine = self_and_ancestors();
    let mut found = Vec::new();
    for entry in std::fs::read_dir("/proc").ok()?.filter_map(Result::ok) {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        if mine.contains(&pid) {
            continue;
        }
        if let Some(comm) = live_comm_of(pid) {
            found.push(ForeignProcess { pid, comm });
        }
    }
    Some(found)
}

/// Every live process descended from `root`, parents before children.
///
/// Follows parent links rather than process groups, so it finds a tool child
/// that left its group through `setsid`; one already reparented to init is
/// beyond it. Blocks on `/proc`.
pub fn descendants_of(root: u32) -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let links: Vec<(u32, u32)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter_map(|pid| Some((pid, parent_of(pid)?)))
        .collect();
    let mut found = vec![root];
    let mut next = 0;
    while next < found.len() {
        let parent = found[next];
        found.extend(
            links
                .iter()
                .filter(|(pid, ppid)| *ppid == parent && !found.contains(pid))
                .map(|(pid, _)| *pid)
                .collect::<Vec<_>>(),
        );
        next += 1;
    }
    found.remove(0);
    found
}

/// One process's resident memory, as an OOM report names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessMemory {
    pub pid: u32,
    pub comm: String,
    pub rss_bytes: u64,
}

/// The `limit` live processes holding the most resident memory, largest first.
///
/// This process is included, unlike [`foreign_live_processes`]: the question
/// is whether the sandbox or the agent's workload holds the memory.
pub fn largest_by_rss(limit: usize) -> Vec<ProcessMemory> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut found: Vec<ProcessMemory> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter_map(|pid| {
            Some(ProcessMemory {
                pid,
                comm: live_comm_of(pid)?,
                rss_bytes: rss_of(pid)?,
            })
        })
        .collect();
    found.sort_by_key(|p| std::cmp::Reverse(p.rss_bytes));
    found.truncate(limit);
    found
}

/// `VmRSS` from `/proc/<pid>/status`, which the kernel prints in KiB; absent
/// for a kernel thread.
fn rss_of(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let kib = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?
        .trim()
        .strip_suffix("kB")?
        .trim()
        .parse::<u64>()
        .ok()?;
    Some(kib * 1024)
}

/// This process and the chain it was started from, which are never writers.
///
/// The chain matters where the sandbox is not PID 1 — a container entrypoint,
/// or a test binary under `cargo` — since a parent that started us is not a
/// writer we have to wait out.
fn self_and_ancestors() -> Vec<u32> {
    let mut chain = Vec::new();
    let mut pid = std::process::id();
    // Bounded rather than `while`: a `/proc` that reports a cycle must not hang
    // the shutdown path.
    for _ in 0..64 {
        chain.push(pid);
        match parent_of(pid) {
            Some(parent) if parent != 0 && !chain.contains(&parent) => pid = parent,
            _ => break,
        }
    }
    chain
}

/// The command name `pid` runs under, or `None` once it is gone or a zombie.
fn live_comm_of(pid: u32) -> Option<String> {
    let stat = read_stat(pid)?;
    let (comm, after_comm) = split_comm(&stat)?;
    (!matches!(after_comm.split_whitespace().next()?, "Z" | "X" | "x")).then(|| comm.to_string())
}

fn parent_of(pid: u32) -> Option<u32> {
    let stat = read_stat(pid)?;
    let (_, after_comm) = split_comm(&stat)?;
    after_comm.split_whitespace().nth(1)?.parse().ok()
}

fn read_stat(pid: u32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()
}

/// The command name, and the fields after it: state, ppid, pgrp, … The name is
/// parenthesised and may itself contain spaces or parens, so it is split off
/// from the last `)` rather than by whitespace.
fn split_comm(stat: &str) -> Option<(&str, &str)> {
    let (before, after) = stat.rsplit_once(')')?;
    Some((before.split_once('(')?.1, after))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tool child an agent detached with `setsid` is still below the agent.
    #[tokio::test]
    async fn descendants_include_a_child_that_left_the_group() {
        let mut parent = tokio::process::Command::new("sh")
            .arg("-c")
            .arg("setsid sleep 30 & echo $!; wait")
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn sh");
        let mut out = tokio::io::BufReader::new(parent.stdout.take().expect("stdout"));
        let mut line = String::new();
        tokio::io::AsyncBufReadExt::read_line(&mut out, &mut line)
            .await
            .expect("child pid");
        let child: u32 = line.trim().parse().expect("pid");
        // `setsid` may fork before exec, so the sleeper is the child or its child.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let found = descendants_of(parent.id().expect("pid"));

        assert!(found.contains(&child), "{found:?} misses {child}");
        for pid in found {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(i32::try_from(pid).expect("pid")),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
    }

    #[test]
    fn this_process_is_never_a_writer_to_wait_out() {
        let found = foreign_live_processes().expect("/proc is readable");
        assert!(
            !found.iter().any(|p| p.pid == std::process::id()),
            "the scanning process listed itself"
        );
    }

    /// A writer that leaves its group and session with `setsid` — which
    /// `agent/options.rs` documents as a supported way to detach — is still a
    /// process in the namespace, which is why certification reads the namespace
    /// rather than the groups this pod handed out.
    #[tokio::test]
    async fn a_detached_child_is_still_a_live_process_in_the_pod() {
        let dir = std::env::temp_dir().join(format!("moat-pod-proc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let pid_file = dir.join("detached.pid");

        // The detached shell reports its own pid: `setsid` forks when its caller
        // leads a group, so the pid of the spawned child is not the writer's.
        tokio::process::Command::new("setsid")
            .arg("sh")
            .arg("-c")
            .arg(format!(
                "echo $$ > {}; exec sleep 30",
                pid_file.to_string_lossy()
            ))
            .process_group(0)
            .spawn()
            .expect("spawn")
            .wait()
            .await
            .expect("wait");
        let pid = read_detached_pid(&pid_file).await;

        let found = foreign_live_processes().expect("/proc is readable");
        assert!(
            found.iter().any(|p| p.pid == pid),
            "the detached writer is missing from {found:?}"
        );

        kill(pid).await;
        assert!(
            !foreign_live_processes()
                .expect("/proc is readable")
                .iter()
                .any(|p| p.pid == pid),
            "a killed writer is not a live process"
        );
    }

    /// An allocation this process touched shows up as its own RSS, ranked
    /// among the pod's largest: the fact an OOM report rests on.
    #[test]
    fn a_process_holding_memory_is_named_with_its_rss() {
        const HELD: usize = 256 * 1024 * 1024;
        let held = vec![1u8; HELD];
        let found = largest_by_rss(usize::MAX);
        let me = found
            .iter()
            .find(|p| p.pid == std::process::id())
            .expect("this process is listed");
        assert!(me.rss_bytes >= HELD as u64, "{me:?}");
        assert!(
            found.windows(2).all(|w| w[0].rss_bytes >= w[1].rss_bytes),
            "not largest first: {found:?}"
        );
        assert_eq!(largest_by_rss(1).len(), 1);
        drop(std::hint::black_box(held));
    }

    async fn read_detached_pid(path: &std::path::Path) -> u32 {
        for _ in 0..200 {
            if let Some(pid) = std::fs::read_to_string(path)
                .ok()
                .and_then(|text| text.trim().parse().ok())
            {
                return pid;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("the detached process never reported its pid");
    }

    /// Shelled out, since this crate denies `unsafe_code`. Waits for the kill to
    /// land, which `kill(1)` only requests.
    async fn kill(pid: u32) {
        tokio::process::Command::new("kill")
            .arg("-KILL")
            .arg(pid.to_string())
            .status()
            .await
            .expect("kill");
        for _ in 0..200 {
            if live_comm_of(pid).is_none() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("the killed writer never went away");
    }
}
