//! Unit tests for the workspace sync loop.

use super::*;

mod worktree;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use sandbox_proto::{
    DownloadUploadResponse, GetCredentialsResponse, GetWorkspaceSyncUrlsResponse, InterruptedTurn,
    PendingAgentDelivery, PublishEventRequest,
};

/// Counts the only call an upload makes before it touches the network, and then
/// declines — so a test can tell "attempted an upload" from "skipped" without
/// standing up an S3 endpoint.
#[derive(Default)]
struct CountingBackend {
    url_requests: AtomicUsize,
    gate: Option<UploadGate>,
    /// Mint URLs pointing at a closed port instead of declining, so a transfer
    /// is reached and then fails.
    mint_urls: bool,
    /// What `get_workspace_sync_urls` fails with, when it should fail with
    /// something other than the default "no S3 here".
    sync_url_code: Option<tonic::Code>,
    /// What `get_workspace_repository_access` fails with. `None` reports the
    /// feature unsupported, like the trait default.
    repository_access_code: Option<tonic::Code>,
    /// When a granted session expires, as RFC 3339. `None` grants nothing.
    repository_expiry: Option<String>,
    repository_access_calls: AtomicUsize,
    /// Every `report_workspace_sync`, as `(format, error)`.
    sync_reports: std::sync::Mutex<Vec<(String, Option<String>)>>,
}

/// Turns the URL mint into a valve: it announces each entry on `started` and
/// then parks until the test hands out a permit, so an upload can be held in
/// flight while the test drives more turns at the tracker.
struct UploadGate {
    started: tokio::sync::mpsc::Sender<()>,
    release: tokio::sync::Semaphore,
}

impl CountingBackend {
    fn attempts(&self) -> usize {
        self.url_requests.load(Ordering::SeqCst)
    }

    /// A backend that blocks inside every upload and then fails the transfer,
    /// plus the receiver that reports each one starting.
    ///
    /// Mints so a released upload answers `NotStored::Failed` and the baseline
    /// stays where a queued upload still reaches the gate.
    fn gated() -> (Arc<Self>, tokio::sync::mpsc::Receiver<()>) {
        let (started, rx) = tokio::sync::mpsc::channel(64);
        let backend = Arc::new(Self {
            gate: Some(UploadGate {
                started,
                release: tokio::sync::Semaphore::new(0),
            }),
            mint_urls: true,
            ..Self::default()
        });
        (backend, rx)
    }

    /// Let `n` blocked uploads through.
    fn release(&self, n: usize) {
        self.gate.as_ref().unwrap().release.add_permits(n);
    }

    /// A backend whose URLs resolve but whose transfers cannot land.
    fn minting() -> Self {
        Self {
            mint_urls: true,
            ..Self::default()
        }
    }

    /// A backend whose repository-access mint fails with `code`, so the
    /// repository arm reports `Failed` rather than an absence.
    fn repository_access_failing(code: tonic::Code) -> Self {
        Self {
            repository_access_code: Some(code),
            ..Self::default()
        }
    }

    /// A backend whose sync-URL mint fails with `code`.
    fn sync_urls_failing(code: tonic::Code) -> Self {
        Self {
            sync_url_code: Some(code),
            ..Self::default()
        }
    }

    /// A backend that grants repository access expiring in `valid_for`.
    fn granting_repository_access(valid_for: chrono::TimeDelta) -> Self {
        Self {
            repository_expiry: Some(
                (chrono::Utc::now() + valid_for).to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            ),
            ..Self::default()
        }
    }

    fn repository_access_calls(&self) -> usize {
        self.repository_access_calls.load(Ordering::SeqCst)
    }

    fn sync_reports(&self) -> Vec<(String, Option<String>)> {
        self.sync_reports.lock().unwrap().clone()
    }
}

#[async_trait]
impl BackendClient for CountingBackend {
    async fn publish_event(&self, _req: PublishEventRequest) -> Result<(), tonic::Status> {
        Ok(())
    }
    async fn get_pending_messages(
        &self,
        _task_id: &str,
    ) -> Result<Vec<PendingAgentDelivery>, tonic::Status> {
        Ok(vec![])
    }
    async fn acknowledge_messages(
        &self,
        _task_id: &str,
        _uuids: Vec<String>,
        _session_id: &str,
        _run_id: &str,
        _answered_prompt: Option<&str>,
    ) -> Result<(), tonic::Status> {
        Ok(())
    }
    async fn get_credentials(
        &self,
        _task_id: &str,
    ) -> Result<GetCredentialsResponse, tonic::Status> {
        Err(tonic::Status::unimplemented("counting"))
    }
    async fn get_system_prompt(&self, _task_id: &str) -> Result<String, tonic::Status> {
        Err(tonic::Status::unimplemented("counting"))
    }
    async fn update_credential(
        &self,
        _task_id: &str,
        _key: &str,
        _value: &str,
    ) -> Result<(), tonic::Status> {
        Ok(())
    }
    async fn download_upload(
        &self,
        _task_id: &str,
        _file_id: &str,
    ) -> Result<DownloadUploadResponse, tonic::Status> {
        Err(tonic::Status::unimplemented("counting"))
    }
    async fn recover_interrupted_run(
        &self,
        _task_id: &str,
    ) -> Result<Option<InterruptedTurn>, tonic::Status> {
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
    async fn get_workspace_sync_urls(
        &self,
        _task_id: &str,
    ) -> Result<GetWorkspaceSyncUrlsResponse, tonic::Status> {
        self.url_requests.fetch_add(1, Ordering::SeqCst);
        if let Some(code) = self.sync_url_code {
            return Err(tonic::Status::new(code, "configured failure"));
        }
        if let Some(gate) = &self.gate {
            let _ = gate.started.try_send(());
            let _permit = gate.release.acquire().await.unwrap();
        }
        if self.mint_urls {
            // Port 1 is bound by nothing, so every transfer is refused
            // without leaving the host.
            return Ok(GetWorkspaceSyncUrlsResponse {
                put_url: "http://127.0.0.1:1/workspace.tar".to_string(),
                get_url: "http://127.0.0.1:1/workspace.tar".to_string(),
                expires_in_secs: 900,
            });
        }
        Err(tonic::Status::failed_precondition("no S3 in tests"))
    }
    async fn report_workspace_sync(
        &self,
        _task_id: &str,
        _direction: &str,
        format: &str,
        _bytes: u64,
        _duration_ms: u64,
        error: Option<&str>,
    ) {
        self.sync_reports
            .lock()
            .unwrap()
            .push((format.to_string(), error.map(str::to_string)));
    }
    async fn get_workspace_repository_access(
        &self,
        _task_id: &str,
    ) -> Result<sandbox_proto::GetWorkspaceRepositoryAccessResponse, tonic::Status> {
        self.repository_access_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(code) = self.repository_access_code {
            return Err(tonic::Status::new(code, "configured failure"));
        }
        let Some(expires_at) = &self.repository_expiry else {
            return Err(tonic::Status::unimplemented(
                "workspace repositories not supported",
            ));
        };
        Ok(sandbox_proto::GetWorkspaceRepositoryAccessResponse {
            bucket: "workspaces".to_string(),
            prefix: "workspaces/task/repo".to_string(),
            region: "eu-north-1".to_string(),
            access_key_id: "key".to_string(),
            secret_access_key: "secret".to_string(),
            session_token: "token".to_string(),
            expires_at: expires_at.clone(),
            password: "password".to_string(),
            source_prefix: None,
            source_password: None,
        })
    }
}

/// A config whose workspace is `dir`, with the two storage switches set.
fn sync_config(dir: &std::path::Path, sync: bool, restore_if_empty: bool) -> Config {
    let mut config = crate::config::test_support::minimal_config();
    config.workspace_dir = dir.to_string_lossy().into_owned();
    config.repo_dir = dir.join("repo").to_string_lossy().into_owned();
    config.workspace_sync_enabled = sync;
    config.workspace_restore_if_empty = restore_if_empty;
    config
}

/// The periodic loop exists to bound the loss from a kill that destroys the
/// only copy — so it runs exactly when the volume is not durable.
#[test]
fn the_periodic_loop_runs_only_without_a_durable_volume() {
    let dir = std::path::Path::new("/tmp/never-read");
    assert!(is_periodic_enabled(&sync_config(dir, true, false)));
    assert!(!is_periodic_enabled(&sync_config(dir, true, true)));
    assert!(!is_periodic_enabled(&sync_config(dir, false, false)));
}

#[tokio::test]
async fn a_committed_repo_is_usable() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    std::fs::write(repo.join("a.txt"), "hello").unwrap();
    git_add(&repo, &["a.txt"]);
    git_commit(&repo, "init");

    assert!(primary_repo_is_usable(&repo).await);
}

#[tokio::test]
async fn an_empty_or_broken_repo_is_not_usable() {
    let tmp = tempfile::tempdir().unwrap();

    // Nothing there at all — a freshly created PVC.
    let missing = tmp.path().join("missing");
    assert!(!primary_repo_is_usable(&missing).await);

    // A directory with files but no `.git` — a half-extracted tar.
    let no_git = tmp.path().join("no-git");
    std::fs::create_dir_all(&no_git).unwrap();
    std::fs::write(no_git.join("a.txt"), "hello").unwrap();
    assert!(!primary_repo_is_usable(&no_git).await);

    // A `.git` that exists but cannot answer `rev-parse HEAD` — an init'd
    // repo with no commit, or one corrupted by a kill mid-write.
    let no_head = tmp.path().join("no-head");
    init_repo(&no_head);
    assert!(!primary_repo_is_usable(&no_head).await);
}

/// The overlay is owed only when nothing authoritative is already in place.
///
/// A durable volume we chose to keep must suppress it exactly like a
/// freshly-extracted snapshot, or `download_on_start` reverts work that never
/// reached a snapshot.
#[test]
fn only_a_bare_decision_still_owes_the_overlay() {
    assert!(
        RestoreDecision::RestoredSnapshot {
            taken_at: None,
            final_flush: false
        }
        .skips_overlay()
    );
    assert!(RestoreDecision::UseExistingVolume.skips_overlay());
    assert!(!RestoreDecision::Nothing.skips_overlay());
    // The overlay extracts the rescue tar. A walk that refused to descend to
    // it must not have it laid over the clone one step later.
    assert!(RestoreDecision::CloneOnly.skips_overlay());

    // Only a real extraction counts as a snapshot origin; keeping the volume
    // lost nothing, so it is not reported as a restore.
    assert!(
        RestoreDecision::RestoredSnapshot {
            taken_at: None,
            final_flush: false
        }
        .restored_snapshot()
    );
    assert!(!RestoreDecision::UseExistingVolume.restored_snapshot());
    assert!(!RestoreDecision::Nothing.restored_snapshot());
    assert!(!RestoreDecision::CloneOnly.restored_snapshot());
}

/// Whether a writer could still start is the first thing the capture asks, and
/// the answer is no only once admission has closed and drained. An open
/// workspace is one a command can be admitted into while the backup runs.
#[tokio::test]
async fn a_capture_over_an_open_workspace_certifies_nothing() {
    let state = quiet_state();
    assert!(!registered_writers_are_quiet(&state).await);

    state
        .writer_admission
        .close_and_drain(Duration::from_secs(5))
        .await;

    assert!(registered_writers_are_quiet(&state).await);
}

/// The registries name only the writers this pod started through them. An
/// unregistered process is the same shape as an agent tool's own child
/// outliving the agent: every registry is quiet and a writer is still live.
#[tokio::test]
async fn a_live_process_no_registry_lists_certifies_nothing() {
    let state = quiet_state();
    state
        .writer_admission
        .close_and_drain(Duration::from_secs(5))
        .await;
    assert!(registered_writers_are_quiet(&state).await);

    // Spawn the foreign process rather than reading whoever launched the
    // binary: `self_and_ancestors` excludes the launch chain, so a pod with
    // nothing else running holds nothing foreign and the assertion below would
    // report the environment.
    let mut foreign = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("spawn a child this test owns");

    let live = crate::pod_processes::foreign_live_processes().expect("/proc is readable");
    let quiet = workspace_writers_are_quiet(&state).await;

    let _ = foreign.kill();
    let _ = foreign.wait();

    assert!(
        live.iter().any(|p| p.pid == foreign.id()),
        "the child this test spawned is in no registry, so it is foreign"
    );
    assert!(!quiet);
}

/// A request admitted before the close and still unfinished at the end of the
/// drain has yet to register the writer the reads below look for, so the
/// capture cannot report the tree whole.
#[tokio::test(start_paused = true)]
async fn a_writer_the_drain_could_not_wait_out_certifies_nothing() {
    let state = quiet_state();
    let _admitted = state.writer_admission.admit().expect("open");

    state
        .writer_admission
        .close_and_drain(Duration::from_millis(50))
        .await;

    assert!(!registered_writers_are_quiet(&state).await);
}

/// A state whose every other writer check already passes, so a test reads only
/// the one it sets.
fn quiet_state() -> crate::state::SandboxState {
    let tmp = tempfile::tempdir().unwrap();
    crate::state::test_support::state_with(
        sync_config(tmp.path(), false, false),
        Arc::new(CountingBackend::default()) as Arc<dyn BackendClient>,
    )
}

/// Only the tag reports a whole tree. Every other decision — including a
/// snapshot restore without it — leaves the agent the note that says to check.
#[test]
fn an_untagged_restore_never_reports_the_tree_whole() {
    assert!(
        RestoreDecision::RestoredSnapshot {
            taken_at: None,
            final_flush: true,
        }
        .restored_final_flush()
    );
    assert!(
        !RestoreDecision::RestoredSnapshot {
            taken_at: None,
            final_flush: false,
        }
        .restored_final_flush()
    );
    assert!(!RestoreDecision::UseExistingVolume.restored_final_flush());
    assert!(!RestoreDecision::Nothing.restored_final_flush());
    assert!(!RestoreDecision::CloneOnly.restored_final_flush());
}

/// The walk itself: a repository that cannot read what it stores neither
/// descends to the tar nor lets the overlay descend for it.
#[tokio::test(start_paused = true)]
async fn a_repository_that_cannot_be_read_neither_descends_nor_overlays() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = sync_config(tmp.path(), true, false);
    config.task_id = "task-1".to_string();

    // The incident's shape: the grant is cancelled, so whether a snapshot
    // exists is unknown rather than answered. The trait default is
    // `unimplemented`, and that reads as an absence instead.
    let backend = Arc::new(CountingBackend::repository_access_failing(
        tonic::Code::Cancelled,
    ));
    let state = crate::state::test_support::state_with(
        config,
        Arc::clone(&backend) as Arc<dyn BackendClient>,
    );

    let decision = restore_on_start(&state).await;

    assert_eq!(
        decision,
        RestoreDecision::CloneOnly,
        "an unread repository must not leave the tar overlay owed"
    );
    assert!(
        decision.skips_overlay(),
        "the overlay extracts the very tar the walk refused"
    );
    assert!(
        !decision.restored_snapshot(),
        "nothing was restored, so no RepositoryRestored event is owed"
    );
    assert!(
        state.workspace_sync.uploads_are_held(),
        "this pod must not overwrite the snapshot it could not read"
    );
    assert_eq!(
        backend.attempts(),
        0,
        "the tar is an older copy, so the walk stops before it"
    );
    assert_eq!(
        backend.repository_access_calls(),
        restic::RESTORE_ATTEMPTS as usize,
        "a blip must be re-tested before it ends the task's durability"
    );
}

/// A restore that failed against a snapshot that exists leaves this pod holding
/// a tree that never held the work; uploading it would replace the only copy.
#[tokio::test]
async fn a_held_upload_never_overwrites_the_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    std::fs::write(repo.join("a.txt"), "hello").unwrap();
    git_add(&repo, &["a.txt"]);
    git_commit(&repo, "init");

    let config = sync_config(tmp.path(), true, true);
    let backend = CountingBackend::default();
    let tracker = WorkspaceSyncTracker::new();

    // Unheld, this tree is new to the backend and ships.
    upload_after_turn(&config, &backend, &tracker).await;
    assert_eq!(backend.attempts(), 1, "an unheld tracker uploads");

    tracker.hold_uploads();
    std::fs::write(repo.join("a.txt"), "edited after the failed restore").unwrap();
    upload_after_turn(&config, &backend, &tracker).await;
    assert_eq!(
        backend.attempts(),
        1,
        "a held tracker ships nothing, whatever changed on disk"
    );
}

/// The gate every upload path shares: nothing travels twice.
#[tokio::test]
async fn an_unchanged_workspace_is_not_re_uploaded() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    std::fs::write(repo.join("a.txt"), "hello").unwrap();
    git_add(&repo, &["a.txt"]);
    git_commit(&repo, "init");

    // This backend serves no repository, so the tar is what the gate is
    // read through. Its transfers are reached and then fail, so the last
    // leg below reads `NotStored::Failed` rather than an absence.
    let config = sync_config(tmp.path(), true, true);
    let backend = CountingBackend::minting();
    let tracker = WorkspaceSyncTracker::new();

    // Init's baseline: the tar the clone/restore produced is whole, so the
    // first turn ships nothing.
    tracker.adopt_current(&config).await;
    upload_after_turn(&config, &backend, &tracker).await;
    assert_eq!(backend.attempts(), 0, "an untouched workspace is skipped");

    // An agent edit moves the fingerprint.
    std::fs::write(repo.join("a.txt"), "edited by the agent").unwrap();
    upload_after_turn(&config, &backend, &tracker).await;
    assert_eq!(backend.attempts(), 1, "a changed workspace is uploaded");

    // The PUT failed, so the baseline must not advance: a transient failure
    // would otherwise silently drop the change.
    upload_after_turn(&config, &backend, &tracker).await;
    assert_eq!(
        backend.attempts(),
        2,
        "a failed upload leaves the change pending for the next attempt"
    );
}

/// The gate covers the repository too. A `restic` snapshot of an unchanged tree
/// walks the workspace, reads the index and commits a snapshot object for no
/// data: on 2026-09-08 that was 1114 of 3095 snapshots.
#[tokio::test]
async fn an_unchanged_workspace_costs_no_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    std::fs::write(repo.join("a.txt"), "hello").unwrap();
    git_add(&repo, &["a.txt"]);
    git_commit(&repo, "init");

    let config = sync_config(tmp.path(), true, true);
    // A grant that opens a repository, so the second half reaches a snapshot
    // rather than only reaching for one.
    let backend = CountingBackend::granting_repository_access(chrono::TimeDelta::hours(1));
    let tracker = WorkspaceSyncTracker::new();

    tracker.adopt_current(&config).await;
    upload_after_turn(&config, &backend, &tracker).await;
    assert_eq!(
        backend.repository_access_calls(),
        0,
        "an untouched workspace is not snapshotted"
    );

    std::fs::write(repo.join("a.txt"), "edited by the agent").unwrap();
    upload_after_turn(&config, &backend, &tracker).await;
    assert_eq!(
        backend.repository_access_calls(),
        1,
        "a changed workspace is snapshotted"
    );
}

/// Verifies [`WorkspaceSyncTracker::store_current`]'s gate: a repository
/// failure holds the baseline back even when the tar succeeded.
#[tokio::test]
async fn a_repository_that_stored_nothing_holds_the_baseline_back() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    std::fs::write(repo.join("a.txt"), "hello").unwrap();
    git_add(&repo, &["a.txt"]);
    git_commit(&repo, "init");

    let config = sync_config(tmp.path(), true, true);
    // The repository fails on its grant, so the tick is the partial failure
    // this test is about.
    let backend = CountingBackend::repository_access_failing(tonic::Code::Cancelled);
    let tracker = WorkspaceSyncTracker::new();

    upload_after_turn(&config, &backend, &tracker).await;
    assert_eq!(backend.repository_access_calls(), 1);

    // Nothing edits the tree again, which is the tick before a stop: the
    // last turn's work is in the tar and not in the repository.
    upload_after_turn(&config, &backend, &tracker).await;
    assert_eq!(
        backend.repository_access_calls(),
        2,
        "a tree the repository missed is offered to it again"
    );
}

/// A backend that serves no repository at all owes this tree no snapshot, so it
/// must not read as a repository that missed one — that would hold the baseline
/// back for the task's whole life and cost a tar every tick.
#[tokio::test]
async fn a_backend_serving_no_repository_leaves_the_gate_working() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    std::fs::write(repo.join("a.txt"), "hello").unwrap();
    git_add(&repo, &["a.txt"]);
    git_commit(&repo, "init");

    let config = sync_config(tmp.path(), true, true);
    // Grants nothing and reports it `Unimplemented`, like a backend with no
    // `workspace_repository` configured.
    let backend = CountingBackend::default();
    let tracker = WorkspaceSyncTracker::new();

    upload_after_turn(&config, &backend, &tracker).await;
    assert_eq!(backend.repository_access_calls(), 1);

    upload_after_turn(&config, &backend, &tracker).await;
    assert_eq!(
        backend.repository_access_calls(),
        1,
        "an unchanged tree costs no second reach for a repository nobody serves"
    );
}

/// A mint that fails transiently stored nothing, so it must not read as a
/// stored tar: a baseline moved on that answer leaves the tree stored nowhere
/// until something edits it.
#[tokio::test]
async fn a_failed_mint_does_not_count_as_a_stored_tar() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    std::fs::write(repo.join("a.txt"), "hello").unwrap();
    git_add(&repo, &["a.txt"]);
    git_commit(&repo, "init");

    let config = sync_config(tmp.path(), true, true);
    // `Cancelled` is a blip rather than a verdict: this backend mints URLs,
    // it just did not this time.
    let backend = CountingBackend::sync_urls_failing(tonic::Code::Cancelled);
    let tracker = WorkspaceSyncTracker::new();

    upload_after_turn(&config, &backend, &tracker).await;
    let first = backend.attempts();
    assert!(first > 0, "the tar reached for its URLs");

    upload_after_turn(&config, &backend, &tracker).await;
    assert!(
        backend.attempts() > first,
        "a tree nothing stored is offered again"
    );
}

/// A grant costs a backend round trip plus an STS mint and lasts an hour, so a
/// sync interval short enough to bound a reclaim's loss must not spend one per
/// tick.
#[tokio::test]
async fn a_repository_grant_is_held_until_it_nears_expiry() {
    let backend = CountingBackend::granting_repository_access(chrono::TimeDelta::hours(1));
    let cache = restic::AccessCache::default();
    assert!(cache.get("task", &backend).await.is_ok());
    assert!(cache.get("task", &backend).await.is_ok());
    assert_eq!(
        backend.repository_access_calls(),
        1,
        "a grant with an hour left is reused"
    );

    // A grant reaches one task's prefix, so another task's operation mints
    // its own.
    assert!(cache.get("other-task", &backend).await.is_ok());
    assert_eq!(backend.repository_access_calls(), 2);

    // The credentials are one of the things that can fail an operation, and
    // a session lasts an hour, so a failure drops them.
    cache.invalidate().await;
    assert!(cache.get("other-task", &backend).await.is_ok());
    assert_eq!(backend.repository_access_calls(), 3);

    // A session inside the refresh margin cannot outlast the snapshot that
    // would open with it.
    let expiring = CountingBackend::granting_repository_access(chrono::TimeDelta::minutes(1));
    let cache = restic::AccessCache::default();
    assert!(cache.get("task", &expiring).await.is_ok());
    assert!(cache.get("task", &expiring).await.is_ok());
    assert_eq!(
        expiring.repository_access_calls(),
        2,
        "a grant near expiry is replaced rather than held"
    );
}

/// A repository that fails still owes the task a durable copy: the tar behind
/// it must still run so a restart finds some copy of the work.
#[tokio::test]
async fn a_failed_repository_falls_back_to_the_tar() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    std::fs::write(repo.join("a.txt"), "hello").unwrap();
    git_add(&repo, &["a.txt"]);
    git_commit(&repo, "init");

    let config = sync_config(tmp.path(), true, true);
    // The grant is cancelled, so the repository reports `Failed` rather than
    // the absence a default backend reports.
    let backend = CountingBackend {
        repository_access_code: Some(tonic::Code::Cancelled),
        ..CountingBackend::minting()
    };
    let tracker = WorkspaceSyncTracker::new();

    // The tar mints its URLs and fails the transfer against a closed port.
    upload_after_turn(&config, &backend, &tracker).await;
    assert_eq!(backend.repository_access_calls(), 1);
    assert_eq!(
        backend.attempts(),
        1,
        "a repository that stored nothing is followed by a tar"
    );

    // Neither landed, so nothing advanced and the next turn retries.
    assert_eq!(tracker.baseline().await, None);
}

/// The turn-boundary upload is gated on the S3 switch, not just on the
/// fingerprint: a sandbox with no snapshot configured must not even ask for
/// URLs.
#[tokio::test]
async fn a_sandbox_without_the_s3_switch_never_uploads() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    std::fs::write(repo.join("a.txt"), "hello").unwrap();
    git_add(&repo, &["a.txt"]);
    git_commit(&repo, "init");

    let config = sync_config(tmp.path(), false, false);
    let backend = CountingBackend::default();
    let tracker = WorkspaceSyncTracker::new();

    upload_after_turn(&config, &backend, &tracker).await;
    assert_eq!(backend.attempts(), 0);
}

/// The turn-boundary upload must not hold up the loop that spawned it, and the
/// turns that arrive while it runs must not each get their own transfer.
#[tokio::test]
async fn turn_uploads_run_off_the_caller_and_coalesce() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    std::fs::write(repo.join("a.txt"), "hello").unwrap();
    git_add(&repo, &["a.txt"]);
    git_commit(&repo, "init");

    let config = Arc::new(sync_config(tmp.path(), true, true));
    let (backend, mut started) = CountingBackend::gated();
    let tracker = WorkspaceSyncTracker::new();

    // First boundary. Returning at all is the point: the upload it starts
    // never finishes on its own.
    spawn_after_turn(config.clone(), backend.clone(), tracker.clone());
    started.recv().await.expect("the first upload started");

    // Three more turns land while that one is stuck. One queues behind it;
    // the other two fold into that one.
    for _ in 0..3 {
        spawn_after_turn(config.clone(), backend.clone(), tracker.clone());
    }
    assert_eq!(
        started.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty),
        "no second transfer starts while the first holds the gate"
    );

    backend.release(8);
    started.recv().await.expect("the queued upload runs");
    assert!(
        tokio::time::timeout(Duration::from_millis(250), started.recv())
            .await
            .is_err(),
        "the three turns behind the first upload share one, not three"
    );
    assert_eq!(backend.attempts(), 2);
}

/// A sandbox with no snapshot configured spawns nothing to skip.
#[tokio::test]
async fn a_sandbox_without_the_s3_switch_spawns_no_upload() {
    let tmp = tempfile::tempdir().unwrap();
    let config = Arc::new(sync_config(tmp.path(), false, false));
    let (backend, mut started) = CountingBackend::gated();
    let tracker = WorkspaceSyncTracker::new();

    spawn_after_turn(config, backend.clone(), tracker);
    assert!(
        tokio::time::timeout(Duration::from_millis(250), started.recv())
            .await
            .is_err(),
        "the switch is off, so no upload is attempted"
    );
    assert_eq!(backend.attempts(), 0);
}

/// A flush that overruns its budget is the one loss no upload path reports: the
/// cancelled future never reaches `report_workspace_sync`, and the pod log
/// holding the `warn!` dies with the pod on the reclaim that caused it.
#[tokio::test(start_paused = true)]
async fn a_timed_out_shutdown_flush_reports_the_loss() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    std::fs::write(repo.join("a.txt"), "hello").unwrap();
    git_add(&repo, &["a.txt"]);
    git_commit(&repo, "init");

    let config = sync_config(tmp.path(), true, false);
    // Never released, so the flush cannot finish inside `SHUTDOWN_FLUSH_TIMEOUT`.
    let (backend, _started) = CountingBackend::gated();
    let tracker = WorkspaceSyncTracker::new();

    flush_workspace(&config, backend.as_ref(), &tracker, true).await;

    let reports = backend.sync_reports();
    assert_eq!(reports.len(), 1, "exactly one report: {reports:?}");
    let (format, error) = &reports[0];
    assert_eq!(format, FORMAT_UNKNOWN);
    assert!(
        error.as_deref().is_some_and(|e| e.contains("budget")),
        "the report carries the loss as an error: {error:?}"
    );
    assert!(
        error
            .as_deref()
            .is_some_and(|e| e.contains("budget in tar (")),
        "the report names the phase the budget ran out in: {error:?}"
    );
    assert_eq!(
        tracker.baseline().await,
        None,
        "a flush that stored nothing leaves the next attempt to retry"
    );
}

/// The one upload that runs whatever the fingerprint says.
///
/// Skipping an unchanged tree here would leave the shutdown tag absent exactly
/// when nothing was lost, and the replacement would warn about a loss that
/// never happened — the opposite of what the tag is for.
#[tokio::test]
async fn the_shutdown_flush_stores_a_tree_the_turn_upload_skips() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    std::fs::write(repo.join("a.txt"), "hello").unwrap();
    git_add(&repo, &["a.txt"]);
    git_commit(&repo, "init");

    let config = sync_config(tmp.path(), true, true);
    let backend = CountingBackend::minting();
    let tracker = WorkspaceSyncTracker::new();

    tracker.adopt_current(&config).await;
    upload_after_turn(&config, &backend, &tracker).await;
    assert_eq!(backend.attempts(), 0, "the turn upload skips it");

    // False, so this also pins that a flush with a writer still live stores the
    // tree and only declines to certify it.
    flush_workspace(&config, &backend, &tracker, false).await;
    assert_eq!(backend.attempts(), 1, "the shutdown flush stores it anyway");
}

/// The tag is a claim about the whole workspace, so it must answer to every
/// writer that outlives the agent, not to the agent alone.
#[test]
fn a_flush_with_a_live_writer_stores_the_tree_and_certifies_nothing() {
    assert!(
        Capture::ShutdownFlush {
            writers_quiet: true
        }
        .may_certify_final_tree()
    );
    assert!(
        !Capture::ShutdownFlush {
            writers_quiet: false
        }
        .may_certify_final_tree()
    );
    assert!(!Capture::Periodic.may_certify_final_tree());

    // Both shutdown captures bypass the fingerprint gate: a live command is a
    // reason to withhold the tag, never a reason to skip storing the tree.
    assert!(
        Capture::ShutdownFlush {
            writers_quiet: false
        }
        .is_shutdown_flush()
    );
    assert!(!Capture::Periodic.is_shutdown_flush());
}

/// A pod killed by a node reclaim never answers the pre-stop RPC, so the first
/// snapshot after a start is the only chance to reclaim what it left.
#[test]
fn the_prune_countdown_claims_the_first_snapshot_of_a_pod() {
    let tracker = WorkspaceSyncTracker::new();
    assert!(tracker.claim_prune(), "the first snapshot prunes");
    for _ in 0..PRUNE_EVERY_SNAPSHOTS {
        assert!(!tracker.claim_prune());
    }
    assert!(tracker.claim_prune(), "and every interval after it");
}

/// The tar is the durable copy wherever no `restic` repository is served, so a
/// lock it archives is restored into the replacement exactly as a snapshot's is.
#[test]
fn a_git_lock_stays_out_of_the_tar() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    std::fs::write(repo.join("main.rs"), b"fn main() {}\n").unwrap();
    git_add(&repo, &["main.rs"]);
    git_commit(&repo, "init");
    std::fs::write(repo.join(".git/index.lock"), b"").unwrap();
    std::fs::write(repo.join(".git/refs/heads/main.lock"), b"").unwrap();

    let mut builder = tar::Builder::new(Vec::new());
    assert!(append_repo(&mut builder, &repo, "repo"));
    let archived = builder.into_inner().unwrap();

    let mut archive = tar::Archive::new(&archived[..]);
    let names: Vec<String> = archive
        .entries()
        .unwrap()
        .flatten()
        .map(|entry| entry.path().unwrap().display().to_string())
        .collect();
    assert!(
        names.iter().any(|name| name == "repo/.git/HEAD"),
        "{names:?}"
    );
    assert!(names.iter().any(|name| name == "repo/main.rs"), "{names:?}");
    // Why this walk replaces `append_dir_all` rather than wrapping it: a `.git`
    // subdirectory holding no files still has to reach the archive.
    assert!(
        names
            .iter()
            .any(|name| name.trim_end_matches('/') == "repo/.git/refs/tags"),
        "{names:?}"
    );
    assert!(
        !names
            .iter()
            .any(|name| name.ends_with("index.lock") || name.ends_with("main.lock")),
        "{names:?}"
    );
}

/// A lock is not workspace content, so catching one must not spend an upload:
/// the gate reads `.git` wholesale and a new file there would otherwise move it.
#[tokio::test]
async fn a_git_lock_does_not_move_the_change_gate() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    std::fs::write(repo.join("main.rs"), b"fn main() {}\n").unwrap();
    git_add(&repo, &["main.rs"]);
    git_commit(&repo, "init");

    let before = workspace_fingerprint(tmp.path().to_path_buf()).await;
    std::fs::write(repo.join(".git/index.lock"), b"").unwrap();
    let after = workspace_fingerprint(tmp.path().to_path_buf()).await;

    assert!(before.is_some());
    assert_eq!(before, after);
}

/// The sweep reads whatever tree the pod booted on, so it has to work with no
/// restore ahead of it — the `pvc` tier, whose volume crossed the kill intact.
#[tokio::test]
async fn the_sweep_clears_a_lock_a_dead_pod_left_on_its_volume() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    std::fs::write(repo.join(".git/index.lock"), b"").unwrap();
    std::fs::write(repo.join("Cargo.lock"), b"").unwrap();

    sweep_git_locks(tmp.path().to_path_buf()).await;

    assert!(!repo.join(".git/index.lock").exists());
    assert!(repo.join(".git/HEAD").exists());
    assert!(repo.join("Cargo.lock").exists(), "not ours to delete");
}

/// A snapshot written before the exclusion still carries a lock, and extracting
/// it puts that lock back on disk. That is what makes the restore, not the
/// volume, the later of the two sources the sweep has to run after.
#[tokio::test]
async fn a_restored_snapshot_reintroduces_a_lock_for_the_sweep_to_clear() {
    let mut archive = tar::Builder::new(Vec::new());
    for (name, data) in [
        ("soap-tm/.git/HEAD", &b"ref: refs/heads/main\n"[..]),
        ("soap-tm/.git/index.lock", &b""[..]),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive.append_data(&mut header, name, data).unwrap();
    }
    let stale_tar = archive.into_inner().unwrap();

    let tmp = tempfile::TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    let repo_dir = ws.join("soap-tm");
    tokio::fs::create_dir_all(&ws).await.unwrap();
    let restored = extract_workspace(
        stale_tar,
        ws.clone(),
        repo_dir.clone(),
        "soap-tm".to_string(),
    )
    .await
    .unwrap();

    // `RestoredSnapshot`, so `download_on_start` and its own sweep are skipped.
    assert!(restored);
    assert!(
        repo_dir.join(".git/index.lock").exists(),
        "restore put it back"
    );

    sweep_git_locks(ws.clone()).await;

    assert!(!repo_dir.join(".git/index.lock").exists());
    assert!(repo_dir.join(".git/HEAD").exists());
}

/// Init a git repo at `dir` with one committed file so `git ls-files` works.
fn init_repo(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "t@t.co"],
        vec!["config", "user.name", "t"],
    ] {
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(&args)
            .output()
            .unwrap();
    }
}

fn git_add(dir: &std::path::Path, paths: &[&str]) {
    let mut args = vec!["add"];
    args.extend_from_slice(paths);
    std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(&args)
        .output()
        .unwrap();
}

fn git_commit(dir: &std::path::Path, message: &str) {
    std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["commit", "-q", "-m", message])
        .output()
        .unwrap();
}

#[tokio::test]
async fn fingerprint_tracks_snapshot_content_and_ignores_build_churn() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    tokio::fs::create_dir_all(&ws).await.unwrap();

    let repo = ws.join("repo");
    init_repo(&repo);
    tokio::fs::write(repo.join(".gitignore"), b"ignored/\n")
        .await
        .unwrap();
    tokio::fs::write(repo.join("main.rs"), b"fn main() {}\n")
        .await
        .unwrap();
    git_add(&repo, &["main.rs", ".gitignore"]);
    git_commit(&repo, "init");
    tokio::fs::create_dir_all(repo.join("ignored"))
        .await
        .unwrap();
    tokio::fs::write(repo.join("ignored").join("build.log"), b"x\n")
        .await
        .unwrap();

    let base = workspace_fingerprint(ws.clone())
        .await
        .expect("workspace has content");
    // Recomputing without changes is stable.
    assert_eq!(base, workspace_fingerprint(ws.clone()).await.unwrap());

    // Churning a gitignored artifact does not move the fingerprint.
    tokio::fs::write(repo.join("ignored").join("build.log"), b"xxxxxxxx\n")
        .await
        .unwrap();
    assert_eq!(base, workspace_fingerprint(ws.clone()).await.unwrap());

    // Editing a tracked file moves it.
    tokio::fs::write(repo.join("main.rs"), b"fn main() { work(); }\n")
        .await
        .unwrap();
    let after_edit = workspace_fingerprint(ws.clone()).await.unwrap();
    assert_ne!(base, after_edit);

    // A new commit (which rewrites `.git`) moves it again.
    git_add(&repo, &["main.rs"]);
    git_commit(&repo, "work");
    let after_commit = workspace_fingerprint(ws.clone()).await.unwrap();
    assert_ne!(after_edit, after_commit);

    // A loose file outside any repo moves it, too.
    tokio::fs::write(ws.join("notes.txt"), b"scratch\n")
        .await
        .unwrap();
    assert_ne!(
        after_commit,
        workspace_fingerprint(ws.clone()).await.unwrap()
    );
}

#[tokio::test]
async fn archive_is_gzipped_and_smaller_than_the_plain_tar() {
    let tmp = tempfile::TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    // Compressible, repetitive source content.
    tokio::fs::write(repo.join("main.rs"), "fn main() {}\n".repeat(500))
        .await
        .unwrap();
    git_add(&repo, &["main.rs"]);

    let archive = build_tar(repo.clone()).await.expect("archive built");
    assert!(
        archive.starts_with(&GZIP_MAGIC),
        "archive must be gzipped for the S3 transfer"
    );

    let plain = decompress_if_gzip(archive.clone()).unwrap();
    assert!(
        archive.len() < plain.len(),
        "gzip must shrink the payload: {} compressed vs {} plain",
        archive.len(),
        plain.len()
    );

    // And it still restores to the same content.
    let dst = tmp.path().join("restored");
    tokio::fs::create_dir_all(&dst).await.unwrap();
    extract_tar(archive, dst.clone()).await.unwrap();
    assert_eq!(
        tokio::fs::read_to_string(dst.join("main.rs"))
            .await
            .unwrap(),
        "fn main() {}\n".repeat(500)
    );
}

#[tokio::test]
async fn an_uncompressed_archive_still_restores() {
    let tmp = tempfile::TempDir::new().unwrap();
    let repo = tmp.path().join("src-repo");
    init_repo(&repo);
    tokio::fs::write(repo.join("a.txt"), b"hi\n").await.unwrap();
    git_add(&repo, &["a.txt"]);

    // A plain (non-gzipped) tar of the repo at the archive root.
    let plain_tar = tokio::task::spawn_blocking({
        let repo = repo.clone();
        move || {
            let mut builder = tar::Builder::new(Vec::new());
            builder.append_dir_all(".", &repo).unwrap();
            builder.into_inner().unwrap()
        }
    })
    .await
    .unwrap();
    assert!(!plain_tar.starts_with(&GZIP_MAGIC));

    let ws = tmp.path().join("ws");
    let repo_dir = ws.join("primary");
    tokio::fs::create_dir_all(&repo_dir).await.unwrap();
    let restored = extract_workspace(
        plain_tar,
        ws.clone(),
        repo_dir.clone(),
        "primary".to_string(),
    )
    .await
    .unwrap();

    assert!(
        restored,
        "an uncompressed snapshot must still be restorable"
    );
    assert!(repo_dir.join("a.txt").exists());
    assert!(repo_dir.join(".git").is_dir());
}

#[tokio::test]
async fn fingerprint_is_none_for_empty_or_missing_workspace() {
    let tmp = tempfile::TempDir::new().unwrap();
    assert!(
        workspace_fingerprint(tmp.path().join("nope"))
            .await
            .is_none()
    );
    let empty = tmp.path().join("empty");
    tokio::fs::create_dir_all(&empty).await.unwrap();
    assert!(workspace_fingerprint(empty).await.is_none());
}

#[test]
fn primary_prefix_derives_relative_repo_name() {
    assert_eq!(primary_prefix("/ws", "/ws/soap-tm"), "soap-tm");
    assert_eq!(primary_prefix("/ws", "/ws"), "");
    // repo_dir not under workspace_dir → empty (treated as root).
    assert_eq!(primary_prefix("/ws", "/other/repo"), "");
}

#[tokio::test]
async fn tar_round_trip_respects_gitignore() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ws = tmp.path().join("repo");
    init_repo(&ws);

    tokio::fs::write(ws.join(".gitignore"), b"ignored/\n")
        .await
        .unwrap();
    tokio::fs::write(ws.join("src.txt"), b"tracked\n")
        .await
        .unwrap();
    tokio::fs::write(ws.join("untracked.txt"), b"untracked\n")
        .await
        .unwrap();
    tokio::fs::create_dir_all(ws.join("ignored")).await.unwrap();
    tokio::fs::write(ws.join("ignored").join("skip.txt"), b"nope\n")
        .await
        .unwrap();
    git_add(&ws, &["src.txt", ".gitignore"]);

    // `ws` itself is a repo → build_tar snapshots it at the archive root.
    let tar = build_tar(ws.clone()).await.expect("tar built");
    assert!(!tar.is_empty());

    let dst = tmp.path().join("restored");
    tokio::fs::create_dir_all(&dst).await.unwrap();
    extract_tar(tar, dst.clone()).await.unwrap();

    // Tracked + untracked-non-ignored are present; the ignored path is not.
    assert!(dst.join("src.txt").exists());
    assert!(dst.join("untracked.txt").exists());
    assert!(!dst.join("ignored").join("skip.txt").exists());
    // The archive carries `.git`, so the restored tree is a real repo.
    assert!(dst.join(".git").is_dir());
}

#[tokio::test]
async fn snapshot_captures_all_data_and_honors_repo_gitignore() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    tokio::fs::create_dir_all(&ws).await.unwrap();

    // Primary repo (a subdir) with a gitignored build artifact + a sibling
    // repo the agent "cloned".
    let primary = ws.join("soap-tm");
    init_repo(&primary);
    tokio::fs::write(primary.join(".gitignore"), b"target/\n")
        .await
        .unwrap();
    tokio::fs::write(primary.join("main.rs"), b"fn main() {}\n")
        .await
        .unwrap();
    tokio::fs::create_dir_all(primary.join("target"))
        .await
        .unwrap();
    tokio::fs::write(primary.join("target").join("app.jar"), b"BINARY\n")
        .await
        .unwrap();
    git_add(&primary, &["main.rs", ".gitignore"]);

    let sibling = ws.join("config-default");
    init_repo(&sibling);
    tokio::fs::write(sibling.join("scenarios.yaml"), b"cases: []\n")
        .await
        .unwrap();
    git_add(&sibling, &["scenarios.yaml"]);

    // Loose data outside any repo: a top-level file and a non-git directory.
    tokio::fs::write(ws.join("notes.txt"), b"scratch\n")
        .await
        .unwrap();
    tokio::fs::create_dir_all(ws.join("artifacts"))
        .await
        .unwrap();
    tokio::fs::write(ws.join("artifacts").join("build.log"), b"ok\n")
        .await
        .unwrap();

    // Snapshot the whole workspace.
    let tar = build_tar(ws.clone()).await.expect("tar built");

    // Restore onto a fresh empty workspace; primary_prefix is the primary
    // repo's subdir name.
    let dst = tmp.path().join("restored-ws");
    tokio::fs::create_dir_all(&dst).await.unwrap();
    let restored = extract_workspace(tar, dst.clone(), dst.join("soap-tm"), "soap-tm".to_string())
        .await
        .unwrap();
    assert!(restored, "primary repo present → skip clone");

    // Both repos restored, each a complete repo.
    assert!(dst.join("soap-tm").join("main.rs").exists());
    assert!(dst.join("soap-tm").join(".git").is_dir());
    assert!(dst.join("config-default").join("scenarios.yaml").exists());
    assert!(dst.join("config-default").join(".git").is_dir());

    // Loose (non-repo) data is captured verbatim.
    assert!(dst.join("notes.txt").exists());
    assert!(dst.join("artifacts").join("build.log").exists());

    // But a gitignored artifact *inside a repo* is excluded.
    assert!(
        !dst.join("soap-tm").join("target").join("app.jar").exists(),
        "gitignored file inside a repo must not be snapshotted"
    );
}

/// A repo commits its own agent-config symlinks (`CLAUDE.md -> AGENTS.md`,
/// `.claude/skills -> ../.agents/skills`). Restoring without them leaves a tree
/// where git reports them deleted, and the deletion gets committed.
#[tokio::test]
async fn a_committed_symlink_survives_a_snapshot_round_trip() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    tokio::fs::create_dir_all(&ws).await.unwrap();

    let repo = ws.join("moatless");
    init_repo(&repo);
    tokio::fs::write(repo.join("AGENTS.md"), b"# guidance\n")
        .await
        .unwrap();
    std::os::unix::fs::symlink("AGENTS.md", repo.join("CLAUDE.md")).unwrap();
    git_add(&repo, &["AGENTS.md", "CLAUDE.md"]);
    git_commit(&repo, "add guidance");

    let tar = build_tar(ws.clone()).await.expect("tar built");

    let dst = tmp.path().join("restored-ws");
    tokio::fs::create_dir_all(&dst).await.unwrap();
    let restored = extract_workspace(
        tar,
        dst.clone(),
        dst.join("moatless"),
        "moatless".to_string(),
    )
    .await
    .unwrap();
    assert!(restored, "primary repo present → skip clone");

    let link = dst.join("moatless").join("CLAUDE.md");
    assert!(
        std::fs::symlink_metadata(&link).unwrap().is_symlink(),
        "committed symlink must restore as a symlink"
    );
    assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("AGENTS.md"));

    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(dst.join("moatless"))
        .args(["status", "--porcelain"])
        .output()
        .unwrap();
    assert!(
        status.stdout.is_empty(),
        "restored repo must be clean, got: {}",
        String::from_utf8_lossy(&status.stdout)
    );
}

#[tokio::test]
async fn repo_relative_archive_restores_to_repo_dir() {
    let tmp = tempfile::TempDir::new().unwrap();

    // Build a repo-relative archive: a single repo tarred at the archive
    // root (top-level `.git`).
    let repo = tmp.path().join("repo-relative-src");
    init_repo(&repo);
    tokio::fs::write(repo.join("a.txt"), b"hi\n").await.unwrap();
    git_add(&repo, &["a.txt"]);
    let repo_relative_tar = build_tar(repo.clone()).await.expect("tar built");

    // Restore with a non-empty primary_prefix (repo lives in a subdir of the
    // workspace) → must unpack onto repo_dir, not workspace_dir.
    let ws = tmp.path().join("ws");
    let repo_dir = ws.join("soap-tm");
    tokio::fs::create_dir_all(&repo_dir).await.unwrap();
    let restored = extract_workspace(
        repo_relative_tar,
        ws.clone(),
        repo_dir.clone(),
        "soap-tm".to_string(),
    )
    .await
    .unwrap();
    assert!(restored);
    assert!(repo_dir.join("a.txt").exists());
    assert!(repo_dir.join(".git").is_dir());
    // Not misplaced at the workspace root.
    assert!(!ws.join("a.txt").exists());
}

#[tokio::test]
async fn working_tree_only_archive_is_declined() {
    // A working-tree-only archive (no `.git`) declines so the caller clones
    // and overlays.
    let mut archive = tar::Builder::new(Vec::new());
    let data = b"working tree\n";
    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    archive
        .append_data(&mut header, "only-file.txt", &data[..])
        .unwrap();
    let working_tree_tar = archive.into_inner().unwrap();

    let tmp = tempfile::TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    let repo_dir = ws.join("soap-tm");
    tokio::fs::create_dir_all(&repo_dir).await.unwrap();
    let restored = extract_workspace(
        working_tree_tar,
        ws.clone(),
        repo_dir.clone(),
        "soap-tm".to_string(),
    )
    .await
    .unwrap();
    assert!(!restored);
    assert!(!repo_dir.join("only-file.txt").exists());
    assert!(!ws.join("only-file.txt").exists());
}

#[tokio::test]
async fn rescue_upload_fails_when_there_is_nothing_to_archive() {
    let tmp = tempfile::TempDir::new().unwrap();
    let missing = tmp.path().join("no-such-workspace");

    let err = rescue_upload(
        missing.to_str().unwrap(),
        "http://127.0.0.1:1/put",
        crate::config::SandboxHttpConfig::default().workspace_transfer,
    )
    .await
    .expect_err("an unarchivable workspace is an error, not an empty success");

    // The caller deletes the volume on Ok, so "nothing to send" must not
    // read as "sent" — and the message has to name the directory, since the
    // rescue pod's log is the only place anyone will look.
    assert!(err.contains("no-such-workspace"), "{err}");
}

#[tokio::test]
async fn rescue_upload_fails_when_the_transfer_does() {
    let tmp = tempfile::TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    tokio::fs::write(repo.join("main.rs"), "fn main() {}\n")
        .await
        .unwrap();
    git_add(&repo, &["main.rs"]);

    // Port 1 is reserved and unbound: the PUT cannot land.
    rescue_upload(
        repo.to_str().unwrap(),
        "http://127.0.0.1:1/put",
        crate::config::SandboxHttpConfig::default().workspace_transfer,
    )
    .await
    .expect_err("a rejected transfer must surface, not be swallowed");
}

/// The framing regression behind `411 Length Required`.
///
/// reqwest sends an empty body with neither `Content-Length` nor
/// `Transfer-Encoding`, and S3 rejects an unframed `PUT`.
#[tokio::test]
async fn an_empty_body_is_still_framed_with_a_content_length() {
    let framing = framing_seen_by_a_server(Vec::new()).await;
    assert_eq!(
        framing.as_deref(),
        Some("content-length: 0"),
        "an unframed PUT is what S3 answers with 411"
    );
}

#[tokio::test]
async fn a_non_empty_body_is_framed_with_its_length() {
    let framing = framing_seen_by_a_server(b"hello".to_vec()).await;
    assert_eq!(framing.as_deref(), Some("content-length: 5"));
}

/// `http_put` one body at a throwaway socket; report the framing header the
/// server actually received, lowercased.
async fn framing_seen_by_a_server(body: Vec<u8>) -> Option<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.expect("accept");
        let mut buf = vec![0u8; 4096];
        let n = sock.read(&mut buf).await.expect("read");
        sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
            .await
            .ok();
        String::from_utf8_lossy(&buf[..n])
            .lines()
            .map(str::to_ascii_lowercase)
            .find(|l| l.starts_with("content-length") || l.starts_with("transfer-encoding"))
    });

    let client = http_client(crate::config::SandboxHttpConfig::default().workspace_transfer)
        .expect("client");
    http_put(&client, &format!("http://127.0.0.1:{port}/obj"), body)
        .await
        .expect("the throwaway server answers 200");
    server.await.expect("server task")
}
