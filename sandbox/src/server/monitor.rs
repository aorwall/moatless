//! Server process monitor — reads filesystem artifacts to determine server state.
//!
//! Monitors externally-managed processes (file-explorer, preview) by checking:
//! - PID files (`{data_dir}/{name}-server.pid`)
//! - Log files (`{data_dir}/{name}-server.log`)
//! - TCP reachability on the configured port

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use tracing::debug;

/// Status of a monitored server process.
#[derive(Debug, Clone, Serialize)]
pub struct ServerStatus {
    pub name: String,
    pub label: String,
    pub port: u16,
    pub builtin: bool,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
}

/// Monitor for a single server process.
pub struct ServerProcessMonitor {
    name: String,
    label: String,
    port: u16,
    builtin: bool,
    data_dir: PathBuf,
}

impl ServerProcessMonitor {
    pub fn new(name: &str, label: Option<&str>, data_dir: &str, port: u16, builtin: bool) -> Self {
        Self {
            name: name.to_string(),
            label: label.unwrap_or(name).to_string(),
            port,
            builtin,
            data_dir: Path::new(data_dir).to_path_buf(),
        }
    }

    /// Get the current status by reading filesystem artifacts and checking TCP reachability.
    pub async fn get_status(&self) -> ServerStatus {
        let pid = self.read_pid().await;

        let is_running = match pid {
            Some(p) => is_process_alive(p),
            None => false,
        };

        let status = if !is_running {
            "stopped".to_string()
        } else if self.port > 0 && is_tcp_reachable(self.port).await {
            "started".to_string()
        } else if self.port > 0 {
            "starting".to_string()
        } else {
            // No port configured — process is alive, report running
            "started".to_string()
        };

        ServerStatus {
            name: self.name.clone(),
            label: self.label.clone(),
            port: self.port,
            builtin: self.builtin,
            status,
            error: None,
            pid,
        }
    }

    /// Get the last N lines of the log file.
    pub async fn get_logs(&self, max_lines: usize) -> String {
        let log_path = self.data_dir.join(format!("{}-server.log", self.name));

        match tokio::fs::read_to_string(&log_path).await {
            Ok(content) => {
                let lines: Vec<&str> = content.lines().collect();
                let start = lines.len().saturating_sub(max_lines);
                lines[start..].join("\n")
            }
            Err(e) => {
                debug!(name = %self.name, error = %e, "Failed to read log file");
                String::new()
            }
        }
    }

    async fn read_pid(&self) -> Option<u32> {
        let path = self.data_dir.join(format!("{}-server.pid", self.name));
        let content = tokio::fs::read_to_string(&path).await.ok()?;
        content.trim().parse().ok()
    }
}

/// Check if a process is alive by checking /proc/{pid} or using kill -0.
fn is_process_alive(pid: u32) -> bool {
    // On Linux, check /proc/{pid}/status
    let proc_path = format!("/proc/{pid}");
    std::path::Path::new(&proc_path).exists()
}

/// Check if a TCP port is reachable on localhost with a 500ms timeout.
async fn is_tcp_reachable(port: u16) -> bool {
    let addr = format!("127.0.0.1:{port}");
    tokio::time::timeout(
        Duration::from_millis(500),
        tokio::net::TcpStream::connect(&addr),
    )
    .await
    .is_ok_and(|r| r.is_ok())
}
