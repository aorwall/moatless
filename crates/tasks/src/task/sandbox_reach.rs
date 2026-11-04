//! The [`SandboxReach`] port over the `tasks`, `task_relations` and
//! `sandboxes` tables and the Workspace grants.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use moatless_core::http_auth::SandboxReach;
use moatless_workspaces::workspace::agent_admin::AgentAdminGrants;
use moka::future::Cache;
use tracing::warn;

use super::dao::PgTaskDao;
use crate::message::dao::MessageDao;

/// How long an `agentAdmin` answer is reused, which bounds how late a
/// Workspace re-pointed at another source loses its grant.
const AGENT_ADMIN_CACHE_TTL: Duration = Duration::from_secs(30);

const MAX_CACHED_ADMIN_ANSWERS: u64 = 10_000;

/// Answers what a sandbox token may reach. Built by
/// [`crate::compose::TaskGraph::sandbox_reach`].
pub struct SandboxReachStore {
    tasks: Arc<PgTaskDao>,
    messages: Arc<dyn MessageDao>,
    agent_admin: Arc<AgentAdminGrants>,
    admin_cache: Cache<String, bool>,
}

impl SandboxReachStore {
    pub(crate) fn new(
        tasks: Arc<PgTaskDao>,
        messages: Arc<dyn MessageDao>,
        agent_admin: Arc<AgentAdminGrants>,
    ) -> Self {
        Self {
            tasks,
            messages,
            agent_admin,
            admin_cache: Cache::builder()
                .time_to_live(AGENT_ADMIN_CACHE_TTL)
                .max_capacity(MAX_CACHED_ADMIN_ANSWERS)
                .build(),
        }
    }

    async fn read_agent_admin(&self, sandbox_id: &str) -> Result<bool, sqlx::Error> {
        match self.tasks.sandbox_workspace(sandbox_id).await? {
            Some(workspace_id) => self.agent_admin.holds(&workspace_id).await,
            None => Ok(false),
        }
    }
}

#[async_trait]
impl SandboxReach for SandboxReachStore {
    async fn task_in_reach(&self, task_id: &str, sandbox_id: &str) -> bool {
        let reached = self.tasks.in_sandbox_reach(task_id, sandbox_id).await;
        reached
            .inspect_err(|e| warn!(task_id, sandbox_id, error = %e, "Sandbox reach lookup failed"))
            .unwrap_or(false)
    }

    async fn message_in_reach(&self, message_id: &str, sandbox_id: &str) -> bool {
        match self.messages.find_by_uuid(message_id).await {
            Ok(Some(message)) => self.task_in_reach(&message.task_id, sandbox_id).await,
            Ok(None) => false,
            Err(e) => {
                warn!(message_id, sandbox_id, error = %e, "Sandbox reach message lookup failed");
                false
            }
        }
    }

    async fn is_agent_admin(&self, sandbox_id: &str) -> bool {
        if let Some(cached) = self.admin_cache.get(sandbox_id).await {
            return cached;
        }
        match self.read_agent_admin(sandbox_id).await {
            Ok(admin) => {
                self.admin_cache.insert(sandbox_id.to_string(), admin).await;
                admin
            }
            Err(e) => {
                warn!(sandbox_id, error = %e, "Agent-admin grant lookup failed");
                false
            }
        }
    }
}
