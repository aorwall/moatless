//! An in-memory `BackendClient` for the replacement lifecycle tests. Its
//! pending-message result, interrupted-Run payload, and session-state archive
//! are fixed per test; `acknowledge_messages` prunes the acked row so a later
//! probe sees it gone — modelling the real ack the startup race hinges on.
//! `enqueue_sandbox_message` appends to the same pending list, so a caller that
//! queues and then dispatches finds the row it just wrote.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use sandbox_proto::{
    DownloadUploadResponse, GetCredentialsResponse, InterruptedTurn, PendingAgentDelivery,
    PublishEventRequest,
};
use tokio::sync::Mutex;

use crate::backend_client::BackendClient;

pub(super) struct ReplayBackend {
    /// `Ok(list)` for a normal probe; `Err(())` to model a backend blip. Behind
    /// a `Mutex` only because `acknowledge_messages` prunes it.
    pending: Mutex<Result<Vec<PendingAgentDelivery>, ()>>,
    /// The interrupted turn `recover_interrupted_run` returns (immutable).
    interrupted: Option<InterruptedTurn>,
    /// The session-state archive `download_transcript` serves (immutable).
    download_tar: Vec<u8>,
    /// Number of `acknowledge_messages` calls that fail (without pruning)
    /// before the RPC starts succeeding — models a transient ack outage.
    ack_failures: AtomicUsize,
    /// Every `enqueue_sandbox_message` as `(idempotency_key, text)`, in call
    /// order. Doubles as the dedup store the real backend keys on.
    queued: Mutex<Vec<(String, String)>>,
    /// Models a backend the pod cannot reach, which forces the direct send.
    enqueue_fails: AtomicBool,
    get_pending_calls: AtomicUsize,
    /// Every `recover_interrupted_run` call's task id, in call order — which
    /// Member the pod asked to recover, and how many times.
    recovered: std::sync::Mutex<Vec<String>>,
}

impl ReplayBackend {
    pub(super) fn new(
        pending: Result<Vec<PendingAgentDelivery>, ()>,
        interrupted: Option<InterruptedTurn>,
    ) -> Arc<Self> {
        Self::build(pending, interrupted, Vec::new())
    }

    /// A backend that serves `tar` from `download_transcript`, with no pending
    /// messages and no interrupted run — for the restore tests.
    pub(super) fn with_archive(tar: Vec<u8>) -> Arc<Self> {
        Self::build(Ok(Vec::new()), None, tar)
    }

    /// A backend that serves both `tar` and an interrupted run, with no pending
    /// message — for the restore-then-replay lifecycle test.
    pub(super) fn with_archive_and_interrupted(
        tar: Vec<u8>,
        interrupted: InterruptedTurn,
    ) -> Arc<Self> {
        Self::build(Ok(Vec::new()), Some(interrupted), tar)
    }

    fn build(
        pending: Result<Vec<PendingAgentDelivery>, ()>,
        interrupted: Option<InterruptedTurn>,
        download_tar: Vec<u8>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pending: Mutex::new(pending),
            interrupted,
            download_tar,
            ack_failures: AtomicUsize::new(0),
            queued: Mutex::new(Vec::new()),
            enqueue_fails: AtomicBool::new(false),
            get_pending_calls: AtomicUsize::new(0),
            recovered: std::sync::Mutex::new(Vec::new()),
        })
    }

    /// Make the next `n` `acknowledge_messages` calls fail (without pruning),
    /// modelling a transient ack outage the dispatcher must retry through.
    pub(super) fn fail_next_acks(&self, n: usize) {
        self.ack_failures.store(n, Ordering::SeqCst);
    }

    /// Make every `enqueue_sandbox_message` fail.
    pub(super) fn fail_enqueue(&self) {
        self.enqueue_fails.store(true, Ordering::SeqCst);
    }

    pub(super) async fn queued(&self) -> Vec<(String, String)> {
        self.queued.lock().await.clone()
    }

    /// Rows still awaiting acknowledgment. A row a dispatch declined to take is
    /// still here; one the agent accepted has been pruned.
    pub(super) async fn pending(&self) -> Vec<PendingAgentDelivery> {
        self.pending.lock().await.clone().unwrap_or_default()
    }

    /// A row that arrives after the backend was built — a person writing while
    /// the pod already holds pending rows of its own.
    pub(super) async fn push_pending(&self, delivery: PendingAgentDelivery) {
        if let Ok(msgs) = &mut *self.pending.lock().await {
            msgs.push(delivery);
        }
    }

    pub(super) fn get_pending_calls(&self) -> usize {
        self.get_pending_calls.load(Ordering::SeqCst)
    }

    pub(super) fn recover_calls(&self) -> usize {
        self.recovered_task_ids().len()
    }

    pub(super) fn recovered_task_ids(&self) -> Vec<String> {
        self.recovered.lock().expect("poisoned").clone()
    }
}

/// Joins every pending row's uuid and text (`\n`-joined) into one delivery per
/// fetch, mirroring `AgentPromptBuilder::build_pending_deliveries`.
///
/// Does not model the real builder's stop at the first `start_fresh` row —
/// honour that separately in a test that sets one.
fn coalesce(rows: Vec<PendingAgentDelivery>) -> Vec<PendingAgentDelivery> {
    let Some(first) = rows.first() else {
        return Vec::new();
    };
    vec![PendingAgentDelivery {
        message_uuids: rows.iter().flat_map(|r| r.message_uuids.clone()).collect(),
        prompt_text: rows
            .iter()
            .map(|r| r.prompt_text.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        prompt_response: rows.iter().find_map(|r| r.prompt_response.clone()),
        ..first.clone()
    }]
}

#[async_trait]
impl BackendClient for ReplayBackend {
    async fn publish_event(&self, _req: PublishEventRequest) -> Result<(), tonic::Status> {
        Ok(())
    }

    async fn get_pending_messages(
        &self,
        _task_id: &str,
    ) -> Result<Vec<PendingAgentDelivery>, tonic::Status> {
        self.get_pending_calls.fetch_add(1, Ordering::SeqCst);
        let rows = match &*self.pending.lock().await {
            Ok(msgs) => msgs.clone(),
            Err(()) => return Err(tonic::Status::unavailable("simulated probe failure")),
        };
        Ok(coalesce(rows))
    }

    async fn acknowledge_messages(
        &self,
        _task_id: &str,
        message_uuids: Vec<String>,
        _session_id: &str,
        _run_id: &str,
        _answered_prompt: Option<&str>,
    ) -> Result<(), tonic::Status> {
        // Model a transient ack outage: fail (without pruning) until the injected
        // failure budget is exhausted, so the row stays pending and the dispatcher
        // must retry — never resubmit.
        if self
            .ack_failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(tonic::Status::unavailable("simulated ack failure"));
        }
        // Prune acked messages so a subsequent probe no longer returns them —
        // exactly what lets the startup race manifest if ordering is wrong.
        if let Ok(msgs) = &mut *self.pending.lock().await {
            msgs.retain(|m| {
                m.message_uuids
                    .iter()
                    .all(|uuid| !message_uuids.contains(uuid))
            });
        }
        Ok(())
    }

    async fn get_credentials(
        &self,
        _task_id: &str,
    ) -> Result<GetCredentialsResponse, tonic::Status> {
        Ok(GetCredentialsResponse {
            credentials: HashMap::new(),
            server_env: HashMap::new(),
            ssh_keys: HashMap::new(),
        })
    }

    async fn update_credential(
        &self,
        _task_id: &str,
        _key: &str,
        _value: &str,
    ) -> Result<(), tonic::Status> {
        Ok(())
    }

    async fn get_system_prompt(&self, _task_id: &str) -> Result<String, tonic::Status> {
        Ok(String::new())
    }

    async fn download_upload(
        &self,
        _task_id: &str,
        _file_id: &str,
    ) -> Result<DownloadUploadResponse, tonic::Status> {
        Err(tonic::Status::unimplemented("not used"))
    }

    async fn recover_interrupted_run(
        &self,
        task_id: &str,
    ) -> Result<Option<InterruptedTurn>, tonic::Status> {
        self.recovered
            .lock()
            .expect("poisoned")
            .push(task_id.to_string());
        Ok(self.interrupted.clone())
    }

    async fn upload_transcript(
        &self,
        _task_id: &str,
        _tar_data: Vec<u8>,
    ) -> Result<(), tonic::Status> {
        Ok(())
    }

    async fn download_transcript(&self, _task_id: &str) -> Result<Vec<u8>, tonic::Status> {
        Ok(self.download_tar.clone())
    }

    /// Queue the message as a pending delivery, so the caller's dispatch finds
    /// the row it just wrote — the production path, where the row is what makes
    /// the turn recoverable.
    async fn enqueue_sandbox_message(
        &self,
        _task_id: &str,
        text: &str,
        idempotency_key: Option<&str>,
    ) -> Result<Option<String>, tonic::Status> {
        if self.enqueue_fails.load(Ordering::SeqCst) {
            return Err(tonic::Status::unavailable("simulated enqueue failure"));
        }
        let key = idempotency_key.unwrap_or(text).to_string();
        let mut queued = self.queued.lock().await;
        if queued.iter().any(|(seen, _)| seen == &key) {
            return Ok(None);
        }
        queued.push((key, text.to_string()));
        let uuid = format!("queued-{}", queued.len());
        if let Ok(msgs) = &mut *self.pending.lock().await {
            msgs.push(PendingAgentDelivery {
                message_uuids: vec![uuid.clone()],
                prompt_text: text.to_string(),
                ..Default::default()
            });
        }
        Ok(Some(uuid))
    }
}
