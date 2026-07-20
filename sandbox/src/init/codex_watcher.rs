//! Watcher for `~/.codex/auth.json` rotations.
//!
//! Codex rotates its OAuth refresh token on every refresh and rewrites
//! `auth.json`. The backend's stored copy goes stale immediately and the next
//! refresh — by another task, the user's laptop, or the same task across pod
//! restarts — fails with `refresh_token_reused`. This watcher pushes the
//! rotated blob back to the backend via `BackendClient::update_credential` so
//! `provider_tokens` always holds the live value.
//!
//! See `Fix 4` of the credential-rotation plan for the full rationale.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::event::ModifyKind;
use notify::{EventKind, RecursiveMode, Watcher};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::backend_client::BackendClient;

/// Credential key used by both `GetCredentialsResponse.credentials` and
/// `UpdateCredentialRequest.key` for the Codex auth blob. Must match
/// `backend::sandbox::grpc_server::CredentialKey::CODEX_AUTH_JSON`.
const CODEX_AUTH_KEY: &str = "CODEX_AUTH_JSON";

/// Coalescing window for filesystem events. Codex may write `auth.json`
/// multiple times during a single refresh (truncate + write, or rename-into);
/// debouncing avoids spamming the backend with intermediate states.
const DEBOUNCE: Duration = Duration::from_millis(500);

/// Handle returned by [`spawn`] so callers can seed the watcher's last-known
/// value. Used by the credential dispatcher: when the backend pushes a
/// rotated `auth.json`, the dispatcher writes the file and immediately calls
/// `set_last_value(new)` so the resulting filesystem event is recognised as
/// a no-op and not echoed back as a fresh `UpdateCredential` push.
#[derive(Clone)]
pub struct WatcherHandle {
    pub(crate) last_value: Arc<Mutex<String>>,
}

impl WatcherHandle {
    /// Replace the watcher's cached "last value" without triggering a sync.
    pub async fn set_last_value(&self, value: String) {
        *self.last_value.lock().await = value;
    }
}

/// Spawn the watcher. Returns immediately; the watcher runs until `cancel`
/// fires.
///
/// `auth_path` is typically `<codex_home_dir>/auth.json`. The directory is
/// watched (not the file) because Codex rewrites the file via atomic rename,
/// which would invalidate a file-level watch handle.
pub fn spawn(
    task_id: String,
    auth_path: PathBuf,
    initial_value: String,
    backend_client: Arc<dyn BackendClient>,
    cancel: CancellationToken,
) -> WatcherHandle {
    let last_value: Arc<Mutex<String>> = Arc::new(Mutex::new(initial_value));
    let handle = WatcherHandle {
        last_value: Arc::clone(&last_value),
    };
    tokio::spawn(async move {
        if let Err(e) = run(task_id, auth_path, last_value, backend_client, cancel).await {
            warn!(error = %e, "Codex auth watcher exited with error");
        }
    });
    handle
}

async fn run(
    task_id: String,
    auth_path: PathBuf,
    last_value: Arc<Mutex<String>>,
    backend_client: Arc<dyn BackendClient>,
    cancel: CancellationToken,
) -> Result<(), String> {
    let watch_dir = auth_path
        .parent()
        .ok_or_else(|| format!("auth_path has no parent: {}", auth_path.display()))?
        .to_path_buf();

    if let Err(e) = tokio::fs::create_dir_all(&watch_dir).await {
        return Err(format!(
            "failed to ensure watch dir {}: {e}",
            watch_dir.display()
        ));
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(32);
    let target_path = auth_path.clone();

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let event = match res {
            Ok(ev) => ev,
            Err(e) => {
                debug!(error = %e, "notify watcher error (transient)");
                return;
            }
        };
        if !event_targets_auth_json(&event, &target_path) {
            return;
        }
        // Best-effort signal — drop on full channel; debounce will pick up the
        // next event anyway.
        let _ = tx.try_send(());
    })
    .map_err(|e| format!("failed to create file watcher: {e}"))?;

    watcher
        .watch(&watch_dir, RecursiveMode::NonRecursive)
        .map_err(|e| format!("failed to start watching {}: {e}", watch_dir.display()))?;

    info!(
        path = %auth_path.display(),
        "Codex auth watcher started"
    );

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                info!("Codex auth watcher shutting down");
                return Ok(());
            }
            received = rx.recv() => {
                if received.is_none() {
                    return Ok(());
                }
                // Debounce: drain any further events that arrive within the window
                // so we only sync once per rotation burst.
                let deadline = tokio::time::sleep(DEBOUNCE);
                tokio::pin!(deadline);
                loop {
                    tokio::select! {
                        () = &mut deadline => break,
                        next = rx.recv() => {
                            if next.is_none() {
                                return Ok(());
                            }
                        }
                    }
                }

                sync_if_changed(&task_id, &auth_path, &last_value, backend_client.as_ref()).await;
            }
        }
    }
}

fn event_targets_auth_json(event: &notify::Event, target: &Path) -> bool {
    // Only act on writes / creates / renames — pure access events don't change content.
    match event.kind {
        EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Name(_) | ModifyKind::Any)
        | EventKind::Create(_) => {}
        _ => return false,
    }
    event.paths.iter().any(|p| p == target)
}

async fn sync_if_changed(
    task_id: &str,
    auth_path: &Path,
    last_value: &Arc<Mutex<String>>,
    backend_client: &dyn BackendClient,
) {
    let new_value = match tokio::fs::read_to_string(auth_path).await {
        Ok(v) => v,
        Err(e) => {
            debug!(
                error = %e,
                path = %auth_path.display(),
                "Codex auth file unreadable — skipping sync"
            );
            return;
        }
    };

    if new_value.trim().is_empty() {
        debug!("Codex auth file empty — skipping sync");
        return;
    }

    {
        let cached = last_value.lock().await;
        if *cached == new_value {
            debug!("Codex auth file unchanged — skipping sync");
            return;
        }
    }

    match backend_client
        .update_credential(task_id, CODEX_AUTH_KEY, &new_value)
        .await
    {
        Ok(()) => {
            let mut cached = last_value.lock().await;
            *cached = new_value;
            info!("Pushed rotated Codex credential to backend");
        }
        Err(e) => {
            warn!(error = %e, "Failed to push rotated Codex credential — will retry on next change");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use sandbox_proto::{GetCredentialsResponse, PendingMessage, PublishEventRequest};
    use tempfile::TempDir;

    use super::*;

    struct RecordingBackend {
        update_count: AtomicUsize,
        last_value: tokio::sync::Mutex<Option<String>>,
    }

    impl RecordingBackend {
        fn new() -> Self {
            Self {
                update_count: AtomicUsize::new(0),
                last_value: tokio::sync::Mutex::new(None),
            }
        }
    }

    #[async_trait::async_trait]
    impl BackendClient for RecordingBackend {
        async fn publish_event(&self, _req: PublishEventRequest) -> Result<(), tonic::Status> {
            Ok(())
        }
        async fn get_pending_messages(
            &self,
            _task_id: &str,
        ) -> Result<Vec<PendingMessage>, tonic::Status> {
            Ok(vec![])
        }
        async fn acknowledge_messages(
            &self,
            _task_id: &str,
            _message_uuids: Vec<String>,
            _session_id: &str,
            _run_id: &str,
        ) -> Result<(), tonic::Status> {
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
            value: &str,
        ) -> Result<(), tonic::Status> {
            self.update_count.fetch_add(1, Ordering::SeqCst);
            *self.last_value.lock().await = Some(value.to_string());
            Ok(())
        }
        async fn get_system_prompt(&self, _task_id: &str) -> Result<String, tonic::Status> {
            Ok(String::new())
        }
        async fn download_upload(
            &self,
            _task_id: &str,
            _file_id: &str,
        ) -> Result<sandbox_proto::DownloadUploadResponse, tonic::Status> {
            Err(tonic::Status::unimplemented("not used"))
        }
        async fn recover_interrupted_run(
            &self,
            _task_id: &str,
        ) -> Result<Option<sandbox_proto::InterruptedTurn>, tonic::Status> {
            Ok(None)
        }

        async fn upload_transcript(
            &self,
            _task_id: &str,
            _tar_data: Vec<u8>,
        ) -> Result<(), tonic::Status> {
            Ok(())
        }

        async fn download_transcript(&self, _task_id: &str) -> Result<Vec<u8>, tonic::Status> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn sync_skips_when_unchanged() {
        let tmp = TempDir::new().unwrap();
        let auth_path = tmp.path().join("auth.json");
        let original = r#"{"tokens":{"refresh_token":"a"}}"#.to_string();
        tokio::fs::write(&auth_path, &original).await.unwrap();

        let backend = Arc::new(RecordingBackend::new());
        let last = Arc::new(Mutex::new(original.clone()));

        sync_if_changed("task-1", &auth_path, &last, backend.as_ref()).await;

        assert_eq!(backend.update_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn sync_pushes_when_content_changes() {
        let tmp = TempDir::new().unwrap();
        let auth_path = tmp.path().join("auth.json");
        let original = r#"{"tokens":{"refresh_token":"a"}}"#.to_string();
        let rotated = r#"{"tokens":{"refresh_token":"b"}}"#.to_string();
        tokio::fs::write(&auth_path, &rotated).await.unwrap();

        let backend = Arc::new(RecordingBackend::new());
        let last = Arc::new(Mutex::new(original));

        sync_if_changed("task-1", &auth_path, &last, backend.as_ref()).await;

        assert_eq!(backend.update_count.load(Ordering::SeqCst), 1);
        let pushed = backend.last_value.lock().await.clone();
        assert_eq!(pushed.as_deref(), Some(rotated.as_str()));
        // Cache updated to the new value.
        let cached = last.lock().await.clone();
        assert_eq!(cached, rotated);
    }

    #[tokio::test]
    async fn set_last_value_suppresses_subsequent_sync() {
        // Simulates the dispatcher path: backend pushes a rotated blob, we
        // write the file AND seed `set_last_value` BEFORE the watcher's debounce
        // fires. The next `sync_if_changed` should see the cache match and not
        // re-push the same value back to the backend (which would echo-loop).
        let tmp = TempDir::new().unwrap();
        let auth_path = tmp.path().join("auth.json");
        let rotated = r#"{"tokens":{"refresh_token":"backend-pushed"}}"#.to_string();
        tokio::fs::write(&auth_path, &rotated).await.unwrap();

        let backend = Arc::new(RecordingBackend::new());
        let last = Arc::new(Mutex::new("seed".to_string()));
        let handle = WatcherHandle {
            last_value: Arc::clone(&last),
        };

        // Dispatcher seeds the cache to match what's now on disk.
        handle.set_last_value(rotated.clone()).await;

        sync_if_changed("task-1", &auth_path, &last, backend.as_ref()).await;

        // No push — the dispatcher's seed prevented echo-back.
        assert_eq!(backend.update_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn sync_skips_empty_file() {
        let tmp = TempDir::new().unwrap();
        let auth_path = tmp.path().join("auth.json");
        tokio::fs::write(&auth_path, "").await.unwrap();

        let backend = Arc::new(RecordingBackend::new());
        let last = Arc::new(Mutex::new("seed".to_string()));

        sync_if_changed("task-1", &auth_path, &last, backend.as_ref()).await;

        assert_eq!(backend.update_count.load(Ordering::SeqCst), 0);
    }
}
