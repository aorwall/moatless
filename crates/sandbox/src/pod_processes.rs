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
