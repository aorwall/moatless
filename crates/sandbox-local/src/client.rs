//! In-memory registry of local sandbox child processes.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use moatless_sandbox_runtime::operation::OperationGeneration;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

/// How long a stopped sandbox gets to stop its agent and flush before SIGKILL.
const STOP_GRACE: Duration = Duration::from_secs(10);

/// One task's local sandbox process and the ports it was told to bind.
pub struct ProcessEntry {
    pub child: Child,
    /// The session id of everything the sandbox started; `Child::id` is gone
    /// once the process has been reaped.
    pub sid: Option<u32>,
    pub http_port: u16,
    pub grpc_port: u16,
    pub generation: Option<OperationGeneration>,
}

/// A snapshot of one entry's liveness and identity, read without holding the
/// registry lock across the caller's own work.
#[derive(Clone, Copy)]
pub struct ProcessSnapshot {
    pub alive: bool,
    pub http_port: u16,
    pub grpc_port: u16,
    pub generation: Option<OperationGeneration>,
}

/// Shared, cheap-to-clone handle onto the local-process registry.
///
/// Cloning shares the same registry (`Arc`-wrapped); `root`/`bin` are
/// immutable per-deployment configuration.
#[derive(Clone)]
pub struct LocalClient {
    processes: Arc<Mutex<HashMap<String, ProcessEntry>>>,
    root: Arc<String>,
    bin: Arc<String>,
}

impl LocalClient {
    pub fn new(root: String, bin: String) -> Self {
        Self {
            processes: Arc::new(Mutex::new(HashMap::new())),
            root: Arc::new(root),
            bin: Arc::new(bin),
        }
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    /// Allocate two free local TCP ports (`http`, `grpc`) by binding and
    /// dropping listeners; another process can still take one before the
    /// sandbox binds it.
    pub async fn allocate_ports() -> std::io::Result<(u16, u16)> {
        let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let http_port = http_listener.local_addr()?.port();
        drop(http_listener);
        let grpc_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let grpc_port = grpc_listener.local_addr()?.port();
        drop(grpc_listener);
        Ok((http_port, grpc_port))
    }

    /// Snapshot whether the task's process is alive, without holding the lock
    /// beyond the read.
    pub async fn snapshot(&self, task_id: &str) -> Option<ProcessSnapshot> {
        let mut guard = self.processes.lock().await;
        let entry = guard.get_mut(task_id)?;
        let alive = matches!(entry.child.try_wait(), Ok(None));
        Some(ProcessSnapshot {
            alive,
            http_port: entry.http_port,
            grpc_port: entry.grpc_port,
            generation: entry.generation,
        })
    }

    /// Spawn the sandbox binary for `task_id`, replacing any prior entry.
    /// Caller must ensure no entry is currently alive for this task.
    pub async fn spawn(
        &self,
        task_id: &str,
        env: &[(String, String)],
        log_path: &Path,
        http_port: u16,
        grpc_port: u16,
        generation: Option<OperationGeneration>,
    ) -> std::io::Result<()> {
        let stdout_file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
            .await?
            .into_std()
            .await;
        let stderr_file = stdout_file.try_clone()?;

        // Its own session, so `kill` can find every process it started: managed
        // commands lead their own process groups, but stay in the session.
        // `setsid` execs in place, since the child leads no group yet.
        let mut cmd = Command::new("setsid");
        cmd.arg(self.bin.as_str());
        cmd.env_clear();
        for (key, value) in env {
            cmd.env(key, value);
        }
        cmd.stdout(stdout_file);
        cmd.stderr(stderr_file);
        cmd.kill_on_drop(true);

        let child = cmd.spawn()?;
        let sid = child.id();
        let mut guard = self.processes.lock().await;
        guard.insert(
            task_id.to_string(),
            ProcessEntry {
                child,
                sid,
                http_port,
                grpc_port,
                generation,
            },
        );
        Ok(())
    }

    /// Stop the task's process and everything in its session, leaving the
    /// registry entry in place (reported as `Stopped` until removed or
    /// respawned). Returns whether it was alive.
    ///
    /// SIGTERM first, as a pod gets, then SIGKILL after [`STOP_GRACE`]. The
    /// registry lock is not held across the grace period.
    pub async fn kill(&self, task_id: &str) -> bool {
        let (sid, was_alive) = {
            let mut guard = self.processes.lock().await;
            let Some(entry) = guard.get_mut(task_id) else {
                return false;
            };
            (entry.sid, matches!(entry.child.try_wait(), Ok(None)))
        };
        let Some(sid) = sid else {
            return false;
        };
        if was_alive {
            self.terminate_leader(task_id, sid).await;
        }

        // Orphans keep the session id after the leader exits, including a
        // crashed sandbox's agents.
        for _ in 0..3 {
            let members = match tokio::task::spawn_blocking(move || session_members(sid)).await {
                Ok(members) => members,
                Err(e) => {
                    tracing::warn!(task_id, error = %e, "Scanning /proc for the sandbox session failed");
                    break;
                }
            };
            if members.is_empty() {
                break;
            }
            signal(&members, "KILL").await;
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        was_alive
    }

    async fn terminate_leader(&self, task_id: &str, pid: u32) {
        signal(&[pid], "TERM").await;
        let deadline = tokio::time::Instant::now() + STOP_GRACE;
        loop {
            let exited = {
                let mut guard = self.processes.lock().await;
                guard
                    .get_mut(task_id)
                    .is_none_or(|entry| !matches!(entry.child.try_wait(), Ok(None)))
            };
            if exited {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                if let Some(entry) = self.processes.lock().await.get_mut(task_id) {
                    let _ = entry.child.start_kill();
                    let _ = entry.child.wait().await;
                }
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Remove the task's registry entry entirely (killing first if alive).
    pub async fn purge(&self, task_id: &str) {
        self.kill(task_id).await;
        let mut guard = self.processes.lock().await;
        guard.remove(task_id);
    }

    /// Snapshot every registered task for the fleet observer.
    pub async fn snapshot_all(&self) -> Vec<(String, ProcessSnapshot)> {
        let mut guard = self.processes.lock().await;
        guard
            .iter_mut()
            .map(|(task_id, entry)| {
                let alive = matches!(entry.child.try_wait(), Ok(None));
                (
                    task_id.clone(),
                    ProcessSnapshot {
                        alive,
                        http_port: entry.http_port,
                        grpc_port: entry.grpc_port,
                        generation: entry.generation,
                    },
                )
            })
            .collect()
    }
}

/// Live processes whose session id is `sid`, read from `/proc`. Zombies are
/// skipped: they are already dead, and only their parent can reap them.
fn session_members(sid: u32) -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| {
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
                return false;
            };
            // `comm` may hold spaces or parentheses, so fields count from the last `)`.
            let Some((_, rest)) = stat.rsplit_once(')') else {
                return false;
            };
            let fields: Vec<&str> = rest.split_whitespace().collect();
            fields.first() != Some(&"Z") && fields.get(3) == Some(&sid.to_string().as_str())
        })
        .collect()
}

/// Send `signal` to each pid; shelled out since this crate denies `unsafe_code`.
async fn signal(pids: &[u32], signal: &str) {
    let _ = Command::new("kill")
        .arg(format!("-{signal}"))
        .args(pids.iter().map(u32::to_string))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sandbox stand-in that starts `sleep` in its own process group, the
    /// way a managed command does, then either waits or exits at once.
    fn fake_sandbox(dir: &Path, then: &str) -> String {
        let bin = dir.join("sandbox.sh");
        std::fs::write(
            &bin,
            format!("#!/bin/bash\nset -m\nsleep 300 &\necho $! > \"$CHILD_PID_FILE\"\n{then}\n"),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        bin.display().to_string()
    }

    async fn spawn_and_read_child(client: &LocalClient, dir: &Path) -> u32 {
        let pid_file = dir.join("child.pid");
        let env = [
            ("PATH".to_string(), std::env::var("PATH").unwrap()),
            ("CHILD_PID_FILE".to_string(), pid_file.display().to_string()),
        ];
        client
            .spawn("sandbox-1", &env, &dir.join("sandbox.log"), 0, 0, None)
            .await
            .unwrap();
        for _ in 0..100 {
            if let Some(pid) = std::fs::read_to_string(&pid_file)
                .ok()
                .and_then(|s| s.trim().parse().ok())
            {
                return pid;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the fake sandbox never started its child");
    }

    fn is_live(pid: u32) -> bool {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|stat| Some(stat.rsplit_once(')')?.1.split_whitespace().next()? != "Z"))
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn kill_stops_a_child_in_its_own_process_group() {
        let dir = tempfile::tempdir().unwrap();
        let client = LocalClient::new(String::new(), fake_sandbox(dir.path(), "wait"));
        let child = spawn_and_read_child(&client, dir.path()).await;

        assert!(client.kill("sandbox-1").await, "the sandbox was running");
        assert!(!is_live(child), "the sandbox's child outlived the stop");
    }

    #[tokio::test]
    async fn kill_stops_what_an_exited_sandbox_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        let client = LocalClient::new(String::new(), fake_sandbox(dir.path(), "exit 0"));
        let child = spawn_and_read_child(&client, dir.path()).await;
        for _ in 0..100 {
            if client.snapshot("sandbox-1").await.is_some_and(|s| !s.alive) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        assert!(
            !client.kill("sandbox-1").await,
            "the sandbox had already exited"
        );
        assert!(
            !is_live(child),
            "the exited sandbox's child is still running"
        );
    }
}
