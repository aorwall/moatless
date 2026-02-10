//! Server registry — manages server process monitors.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tracing::{info, warn};

use super::monitor::{ServerProcessMonitor, ServerStatus};
use crate::config::ServerConfig;

/// Registry of all server process monitors.
pub struct ServerRegistry {
    monitors: HashMap<String, ServerProcessMonitor>,
    /// Cached statuses from the background monitor loop.
    cached_statuses: Arc<RwLock<HashMap<String, ServerStatus>>>,
}

impl ServerRegistry {
    /// Create a new registry and initialize monitors from server configs.
    pub fn new(servers: &[ServerConfig], data_dir: &str) -> Self {
        let mut monitors = HashMap::new();

        // Always register default servers (builtin)
        let default_servers: Vec<(&str, &str, u16)> = vec![
            ("files", "File Explorer", 4000),
            ("preview", "Preview", 3000),
        ];

        for (name, label, port) in default_servers {
            monitors.insert(
                name.to_string(),
                ServerProcessMonitor::new(name, Some(label), data_dir, port, true),
            );
        }

        // Register configured servers (may override defaults)
        for server in servers {
            monitors.insert(
                server.name.clone(),
                ServerProcessMonitor::new(
                    &server.name,
                    server.label.as_deref(),
                    data_dir,
                    server.port.unwrap_or(0),
                    server.builtin.unwrap_or(false),
                ),
            );
        }

        Self {
            monitors,
            cached_statuses: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Spawn a background task that polls server statuses every 3 seconds
    /// and logs any status changes at info level.
    pub fn spawn_monitor_loop(self: &Arc<Self>) {
        let registry = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                registry.poll_and_update_statuses().await;
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        });
    }

    /// Poll all monitors and update the cached status map, logging changes.
    async fn poll_and_update_statuses(&self) {
        for (name, monitor) in &self.monitors {
            let new_status = match tokio::time::timeout(
                Duration::from_secs(1),
                monitor.get_status(),
            )
            .await
            {
                Ok(status) => status,
                Err(_) => {
                    warn!(server = %name, "Server status poll timed out after 1s");
                    continue;
                }
            };

            let mut cache = self.cached_statuses.write().await;
            let changed = match cache.get(name) {
                Some(prev) => prev.status != new_status.status,
                None => true, // First poll — always insert
            };

            if changed {
                if let Some(prev) = cache.get(name) {
                    info!(
                        server = %name,
                        port = new_status.port,
                        status = %new_status.status,
                        prev_status = %prev.status,
                        "Server status changed"
                    );
                } else {
                    info!(
                        server = %name,
                        port = new_status.port,
                        status = %new_status.status,
                        "Server initial status"
                    );
                }
                cache.insert(name.clone(), new_status);
            }
        }
    }

    /// Get status for a specific server (reads from cache).
    pub async fn get_status(&self, name: &str) -> Option<ServerStatus> {
        self.cached_statuses.read().await.get(name).cloned()
    }

    /// Get status for all servers (reads from cache, instant).
    pub async fn get_all_statuses(&self) -> Vec<ServerStatus> {
        self.cached_statuses.read().await.values().cloned().collect()
    }

    /// Get logs for a specific server.
    pub async fn get_logs(&self, name: &str, max_lines: usize) -> Option<String> {
        let monitor = self.monitors.get(name)?;
        Some(monitor.get_logs(max_lines).await)
    }
}
