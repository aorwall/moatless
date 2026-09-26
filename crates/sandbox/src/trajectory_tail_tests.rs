use std::fmt::Write as _;
use std::io::Write as _;

use super::*;
use crate::agent::harness::trajectory::ClaudeSessionFormat;
use crate::backend_client::BackendClient;

use super::memory::MemoryTrajectoryBackend;

const ID: &str = "0199aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee";

fn session_file(home: &std::path::Path, cwd: &std::path::Path) -> PathBuf {
    let path = ClaudeSessionFormat.session_path(home, cwd, ID, "");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    path
}

fn recorder(
    backend: Arc<dyn BackendClient>,
    home: &std::path::Path,
    cwd: &std::path::Path,
) -> (SessionRecorder, Arc<TrajectoryRecording>) {
    let recording = TrajectoryRecording::new(backend);
    let target = RecordTarget {
        task_id: "task".into(),
        harness_session_id: ID.into(),
        home: home.to_path_buf(),
        cwd: cwd.to_path_buf(),
    };
    let recorder = SessionRecorder::start(
        Arc::new(ClaudeSessionFormat),
        Arc::clone(&recording),
        target,
        None,
    );
    (recorder, recording)
}

#[tokio::test]
async fn record_holds_a_partial_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_file(dir.path(), dir.path());
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let (mut recorder, _) = recorder(backend.clone(), dir.path(), dir.path());

    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(b"{\"type\":\"user\"}\n{\"type\":\"assist")
        .unwrap();
    assert_eq!(recorder.flush().await.unwrap(), 0);
    assert_eq!(backend.events_on(ID).len(), 1);

    assert_eq!(recorder.flush().await.unwrap(), 0);
    assert_eq!(backend.events_on(ID).len(), 1, "the half line is held");

    file.write_all(b"ant\"}\n").unwrap();
    assert_eq!(recorder.flush().await.unwrap(), 1);
    let payloads: Vec<String> = backend
        .events_on(ID)
        .into_iter()
        .map(|e| e.payload)
        .collect();
    assert_eq!(
        payloads,
        ["{\"type\":\"user\"}", "{\"type\":\"assistant\"}"]
    );
}

#[tokio::test]
async fn a_restarted_recorder_skips_what_is_held() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_file(dir.path(), dir.path());
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    std::fs::write(&path, "{\"n\":0}\n{\"n\":1}\n").unwrap();
    let (mut first, _) = recorder(backend.clone(), dir.path(), dir.path());
    first.flush().await.unwrap();

    std::fs::write(&path, "{\"n\":0}\n{\"n\":1}\n{\"n\":2}\n").unwrap();
    let (mut second, _) = recorder(backend.clone(), dir.path(), dir.path());
    assert_eq!(second.flush().await.unwrap(), 2);
    assert_eq!(backend.events_on(ID).len(), 3);

    std::fs::write(&path, "{\"n\":0}\n").unwrap();
    let (mut behind, _) = recorder(backend.clone(), dir.path(), dir.path());
    assert!(matches!(
        behind.flush().await,
        Err(RecordError::Behind {
            held: 1,
            expected: 3
        })
    ));
}

#[tokio::test]
async fn a_diverged_file_is_not_spliced_in() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_file(dir.path(), dir.path());
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    std::fs::write(&path, "{\"n\":0}\n{\"n\":1}\n{\"n\":2}\n").unwrap();
    recorder(backend.clone(), dir.path(), dir.path())
        .0
        .flush()
        .await
        .unwrap();

    // Restored at one line, then resumed past the trajectory's count.
    std::fs::write(&path, "{\"n\":0}\n{\"m\":1}\n{\"m\":2}\n{\"m\":3}\n").unwrap();
    let (mut diverged, _) = recorder(backend.clone(), dir.path(), dir.path());
    assert!(matches!(
        diverged.flush().await,
        Err(RecordError::Diverged { seq: 2 })
    ));
    assert!(matches!(diverged.flush().await, Err(RecordError::Off)));
    assert_eq!(
        backend.events_on(ID).len(),
        3,
        "nothing past the fork in history"
    );
}

#[tokio::test]
async fn recorder_stops_when_recording_is_off() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_file(dir.path(), dir.path());
    std::fs::write(&path, "{\"type\":\"user\"}\n").unwrap();
    let backend = Arc::new(MemoryTrajectoryBackend::off());
    let (mut recorder, recording) = recorder(backend.clone(), dir.path(), dir.path());

    assert!(matches!(recorder.flush().await, Err(RecordError::Off)));
    assert!(recording.is_off());
    let calls = backend.append_calls();

    let sibling = SessionRecorder::start(
        Arc::new(ClaudeSessionFormat),
        Arc::clone(&recording),
        RecordTarget {
            task_id: "task".into(),
            harness_session_id: "other".into(),
            home: dir.path().to_path_buf(),
            cwd: dir.path().to_path_buf(),
        },
        Some(0),
    );
    let live = LiveRecorder::spawn(sibling);
    live.flush().await;
    assert!(matches!(recorder.flush().await, Err(RecordError::Off)));
    assert_eq!(backend.append_calls(), calls, "no recorder calls again");
}

#[tokio::test]
async fn a_file_past_the_batch_budget_ships_in_bounded_batches() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_file(dir.path(), dir.path());
    let filler = "x".repeat(64 * 1024);
    let mut body = String::new();
    for n in 0..40 {
        writeln!(body, "{{\"n\":{n},\"f\":\"{filler}\"}}").unwrap();
    }
    std::fs::write(&path, format!("{body}{{\"partial\"")).unwrap();
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let (mut recorder, _) = recorder(backend.clone(), dir.path(), dir.path());

    assert_eq!(recorder.flush().await.unwrap(), 39);
    assert_eq!(backend.events_on(ID).len(), 40);
    assert!(
        backend.append_calls() >= 3,
        "2.5 MiB goes out as a probe plus at least two batches"
    );
    assert_eq!(
        recorder.offset,
        body.len() as u64,
        "the partial line is held"
    );
}
