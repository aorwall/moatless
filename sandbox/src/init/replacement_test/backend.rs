//! An in-memory `BackendClient` for the replacement lifecycle tests. Its
//! pending-message result, interrupted-Run payload, and session-state archive
//! are fixed per test; `acknowledge_messages` prunes the acked row so a later
//! probe sees it gone — modelling the real ack the startup race hinges on.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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
    /// Number of `acknowledge_messages` calls that fail (without pruning) before
    /// the RPC starts succeeding — models a transient ack outage.
    ack_failures: AtomicUsize,
    get_pending_calls: AtomicUsize,
    recover_calls: AtomicUsize,
}

impl ReplayBackend {
    pub(super) fn new(
        pending: Result<Vec<PendingAgentDelivery>, ()>,
        interrupted: Option<InterruptedTurn>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pending: Mutex::new(pending),
            interrupted,
            download_tar: Vec::new(),
            ack_failures: AtomicUsize::new(0),
            get_pending_calls: AtomicUsize::new(0),
            recover_calls: AtomicUsize::new(0),
        })
    }

    /// A backend that serves `tar` from `download_transcript`, with no pending
    /// messages and no interrupted run — for the restore tests.
    pub(super) fn with_archive(tar: Vec<u8>) -> Arc<Self> {
        Arc::new(Self {
            pending: Mutex::new(Ok(Vec::new())),
            interrupted: None,
            download_tar: tar,
            ack_failures: AtomicUsize::new(0),
            get_pending_calls: AtomicUsize::new(0),
            recover_calls: AtomicUsize::new(0),
        })
    }

    /// A backend that serves both `tar` and an interrupted run, with no pending
    /// message — for the restore-then-replay lifecycle test.
    pub(super) fn with_archive_and_interrupted(
        tar: Vec<u8>,
        interrupted: InterruptedTurn,
    ) -> Arc<Self> {
        Arc::new(Self {
            pending: Mutex::new(Ok(Vec::new())),
            interrupted: Some(interrupted),
            download_tar: tar,
            ack_failures: AtomicUsize::new(0),
            get_pending_calls: AtomicUsize::new(0),
            recover_calls: AtomicUsize::new(0),
        })
    }

    /// Make the next `n` `acknowledge_messages` calls fail (without pruning),
    /// modelling a transient ack outage the dispatcher must retry through.
    pub(super) fn fail_next_acks(&self, n: usize) {
        self.ack_failures.store(n, Ordering::SeqCst);
    }

    pub(super) fn get_pending_calls(&self) -> usize {
        self.get_pending_calls.load(Ordering::SeqCst)
    }

    pub(super) fn recover_calls(&self) -> usize {
        self.recover_calls.load(Ordering::SeqCst)
    }
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
        match &*self.pending.lock().await {
            Ok(msgs) => Ok(msgs.clone()),
            Err(()) => Err(tonic::Status::unavailable("simulated probe failure")),
        }
    }

    async fn acknowledge_messages(
        &self,
        _task_id: &str,
        message_uuids: Vec<String>,
        _session_id: &str,
        _run_id: &str,
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
        _task_id: &str,
    ) -> Result<Option<InterruptedTurn>, tonic::Status> {
        self.recover_calls.fetch_add(1, Ordering::SeqCst);
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
}
