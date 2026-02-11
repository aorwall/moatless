//! Shared Redis sandbox state cache (provider-agnostic).
//!
//! Stores `CachedSandboxState` per task in a single Redis Hash
//! (`moatless:sandbox:state`). The pod_watcher is the sole writer
//! (single-writer pattern); all replicas read via `StatusResolver`.
//!
//! Writers: pod lifecycle watcher (full state), lifecycle operations (delete on cleanup).
//! Readers: StatusResolver (any replica).

use std::collections::HashMap;

use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use tracing::{debug, warn};

use crate::sandbox::types::{
    AgentState, CachedSandboxState, EnvironmentStatus, ServerSnapshot, WorkspaceState,
};

/// Redis Hash-backed sandbox state cache shared across replicas.
#[derive(Clone)]
pub struct SandboxStateCache {
    conn: ConnectionManager,
}

impl SandboxStateCache {
    const HASH_KEY: &str = "moatless:sandbox:state";
    const FRESHNESS_SECS: i64 = 10;

    pub fn new(conn: ConnectionManager) -> Self {
        Self { conn }
    }

    /// Get cached state for a task (regardless of freshness).
    pub async fn get(&self, task_id: &str) -> Option<CachedSandboxState> {
        let mut conn = self.conn.clone();
        let result: Result<Option<String>, redis::RedisError> =
            conn.hget(Self::HASH_KEY, task_id).await;

        match result {
            Ok(Some(json)) => match serde_json::from_str(&json) {
                Ok(state) => Some(state),
                Err(e) => {
                    warn!(task_id, error = %e, "Failed to deserialize cached sandbox state");
                    None
                }
            },
            Ok(None) => None,
            Err(e) => {
                warn!(task_id, error = %e, "Failed to read sandbox state from Redis");
                None
            }
        }
    }

    /// Get cached state only if it was updated within the freshness window.
    pub async fn get_if_fresh(&self, task_id: &str) -> Option<CachedSandboxState> {
        let state = self.get(task_id).await?;
        let now_ms = chrono::Utc::now().timestamp_millis();
        if (now_ms - state.updated_at) < Self::FRESHNESS_SECS * 1000 {
            Some(state)
        } else {
            None
        }
    }

    /// Write full state for a task (fire-and-forget on error).
    pub async fn set(&self, task_id: &str, state: &CachedSandboxState) {
        let json = match serde_json::to_string(state) {
            Ok(j) => j,
            Err(e) => {
                warn!(task_id, error = %e, "Failed to serialize sandbox state");
                return;
            }
        };

        let mut conn = self.conn.clone();
        let result: Result<(), redis::RedisError> =
            conn.hset(Self::HASH_KEY, task_id, &json).await;

        if let Err(e) = result {
            warn!(task_id, error = %e, "Failed to write sandbox state to Redis");
        } else {
            debug!(task_id, "Sandbox state cached in Redis");
        }
    }

    /// Initialize or overwrite the cache entry with environment status and servers.
    ///
    /// Used on provision (Initializing + stopped servers) and reconciliation
    /// (Stopped + stopped servers). Always writes the provided servers —
    /// never leaves them empty.
    pub async fn init_state(
        &self,
        task_id: &str,
        environment_status: EnvironmentStatus,
        servers: Vec<ServerSnapshot>,
    ) {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let state = CachedSandboxState {
            environment_status,
            environment_error: None,
            workspace_state: WorkspaceState::Exists,
            pod_name: None,
            agent_state: AgentState::None,
            servers,
            resources: None,
            is_running: false,
            updated_at: now_ms,
        };
        self.set(task_id, &state).await;
    }

    /// Delete the task's entry from the Hash (cache invalidation).
    pub async fn delete(&self, task_id: &str) {
        let mut conn = self.conn.clone();
        let result: Result<(), redis::RedisError> =
            conn.hdel(Self::HASH_KEY, task_id).await;

        if let Err(e) = result {
            warn!(task_id, error = %e, "Failed to delete sandbox state from Redis");
        } else {
            debug!(task_id, "Sandbox state deleted from Redis");
        }
    }

    /// Get all cached sandbox states (HGETALL).
    /// Used by leader reconciliation to compare Redis state against live K8s state.
    pub async fn get_all(&self) -> HashMap<String, CachedSandboxState> {
        let mut conn = self.conn.clone();
        let result: Result<HashMap<String, String>, redis::RedisError> =
            conn.hgetall(Self::HASH_KEY).await;

        match result {
            Ok(entries) => {
                let mut map = HashMap::with_capacity(entries.len());
                for (task_id, json) in entries {
                    match serde_json::from_str::<CachedSandboxState>(&json) {
                        Ok(state) => {
                            map.insert(task_id, state);
                        }
                        Err(e) => {
                            warn!(task_id, error = %e, "Failed to deserialize cached sandbox state in get_all");
                        }
                    }
                }
                map
            }
            Err(e) => {
                warn!(error = %e, "Failed to HGETALL sandbox states from Redis");
                HashMap::new()
            }
        }
    }

    /// Batch get cached states for multiple tasks (single HMGET round trip).
    pub async fn get_many(&self, task_ids: &[&str]) -> HashMap<String, CachedSandboxState> {
        if task_ids.is_empty() {
            return HashMap::new();
        }

        let mut conn = self.conn.clone();
        let result: Result<Vec<Option<String>>, redis::RedisError> = redis::cmd("HMGET")
            .arg(Self::HASH_KEY)
            .arg(task_ids)
            .query_async(&mut conn)
            .await;

        match result {
            Ok(values) => {
                let mut map = HashMap::with_capacity(values.len());
                for (i, maybe_json) in values.into_iter().enumerate() {
                    if let Some(json) = maybe_json {
                        match serde_json::from_str::<CachedSandboxState>(&json) {
                            Ok(state) => {
                                map.insert(task_ids[i].to_string(), state);
                            }
                            Err(e) => {
                                warn!(task_id = task_ids[i], error = %e, "Failed to deserialize cached sandbox state");
                            }
                        }
                    }
                }
                map
            }
            Err(e) => {
                warn!(error = %e, "Failed to batch read sandbox states from Redis");
                HashMap::new()
            }
        }
    }

    /// Full state update: environment + servers in a single Redis write.
    ///
    /// Used by the pod_watcher to write both environment and server container
    /// state atomically. Preserves existing servers if the new list is empty.
    pub async fn update_full_state(
        &self,
        task_id: &str,
        environment_status: EnvironmentStatus,
        pod_name: Option<String>,
        environment_error: Option<String>,
        servers: Vec<crate::sandbox::types::ServerSnapshot>,
    ) {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let is_ready = environment_status == EnvironmentStatus::Ready;

        let state = if let Some(mut existing) = self.get(task_id).await {
            existing.environment_status = environment_status;
            existing.environment_error = environment_error;
            existing.pod_name = pod_name;
            existing.updated_at = now_ms;
            // Only update servers if we have data; preserve existing otherwise
            if !servers.is_empty() {
                existing.servers = servers;
            }
            // Clear runtime fields when not ready
            if !is_ready {
                existing.agent_state = AgentState::None;
                existing.resources = None;
                existing.is_running = false;
            }
            existing
        } else {
            CachedSandboxState {
                environment_status,
                environment_error,
                workspace_state: if is_ready {
                    WorkspaceState::Exists
                } else {
                    WorkspaceState::Missing
                },
                pod_name,
                agent_state: AgentState::None,
                servers,
                resources: None,
                is_running: false,
                updated_at: now_ms,
            }
        };

        self.set(task_id, &state).await;
    }

    /// Partial update: only environment-related fields.
    ///
    /// Reads current state, merges environment fields, writes back.
    /// If no existing state, creates a minimal entry. Used by the pod
    /// watcher and any provider that detects environment changes.
    pub async fn update_environment_state(
        &self,
        task_id: &str,
        environment_status: EnvironmentStatus,
        pod_name: Option<String>,
        environment_error: Option<String>,
    ) {
        let is_running = environment_status == EnvironmentStatus::Ready;
        let now_ms = chrono::Utc::now().timestamp_millis();

        let state = if let Some(mut existing) = self.get(task_id).await {
            existing.environment_status = environment_status;
            existing.environment_error = environment_error;
            existing.pod_name = pod_name;
            existing.updated_at = now_ms;

            // If pod is no longer running, clear runtime fields
            if !is_running {
                existing.agent_state = AgentState::None;
                existing.servers = vec![];
                existing.resources = None;
                existing.is_running = false;
            }

            existing
        } else {
            CachedSandboxState {
                environment_status,
                environment_error,
                workspace_state: if is_running {
                    WorkspaceState::Exists
                } else {
                    WorkspaceState::Missing
                },
                pod_name,
                agent_state: AgentState::None,
                servers: vec![],
                resources: None,
                is_running: false,
                updated_at: now_ms,
            }
        };

        self.set(task_id, &state).await;
    }
}
