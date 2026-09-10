//! The production `JobInjector`: a scheduled job's prompt is queued with the
//! backend and then dispatched, so the turn it opens owns a recoverable `user`
//! row. See `crates/sandbox/AGENTS.md` "Surviving a pod replacement".

use crate::agent::dispatcher::queue_and_dispatch;
use crate::agent::scheduled_job::JobInjector;
use crate::state::SandboxState;

pub struct QueuedInjector {
    task_id: String,
    state: SandboxState,
}

impl QueuedInjector {
    pub fn new(task_id: String, state: SandboxState) -> Self {
        Self { task_id, state }
    }
}

#[async_trait::async_trait]
impl JobInjector for QueuedInjector {
    async fn inject(&self, idempotency_key: &str, text: String) {
        queue_and_dispatch(&self.state, &self.task_id, text, Some(idempotency_key)).await;
    }
}
