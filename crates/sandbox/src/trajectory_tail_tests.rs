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
        subagent: None,
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
    assert_eq!(behind.flush().await.unwrap(), 2, "reseeded, then placed");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "{\"n\":0}\n{\"n\":1}\n{\"n\":2}\n"
    );
}

#[tokio::test]
async fn a_diverged_file_is_reseeded_from_the_trajectory() {
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
    assert_eq!(diverged.flush().await.unwrap(), 2);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "{\"n\":0}\n{\"n\":1}\n{\"n\":2}\n",
        "nothing past the fork in history"
    );

    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{\"n\":3}\n")
        .unwrap();
    assert_eq!(diverged.flush().await.unwrap(), 3);
    assert_eq!(backend.events_on(ID).len(), 4);
    assert!(backend.published().is_empty());
}

#[tokio::test]
async fn a_failed_reseed_stops_the_recorder_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_file(dir.path(), dir.path());
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    std::fs::write(&path, "{\"n\":0}\n{\"n\":1}\n{\"n\":2}\n").unwrap();
    recorder(backend.clone(), dir.path(), dir.path())
        .0
        .flush()
        .await
        .unwrap();
    backend.lose(ID, 1);

    std::fs::write(&path, "{\"n\":0}\n").unwrap();
    let (mut behind, _) = recorder(backend.clone(), dir.path(), dir.path());
    assert!(matches!(
        behind.flush().await,
        Err(RecordError::Behind { .. })
    ));
    assert!(matches!(behind.flush().await, Err(RecordError::Off)));
    let published = backend.published();
    assert_eq!(published.len(), 1);
    assert_eq!(published[0].0, "recording_stopped");
    assert!(published[0].1.contains(ID));
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
            subagent: None,
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

#[tokio::test]
async fn an_append_carries_at_most_the_line_cap() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_file(dir.path(), dir.path());
    let mut body = String::new();
    for n in 0..600 {
        writeln!(body, "{{\"n\":{n}}}").unwrap();
    }
    std::fs::write(&path, body).unwrap();
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let (mut recorder, _) = recorder(backend.clone(), dir.path(), dir.path());

    assert_eq!(recorder.flush().await.unwrap(), 599);
    assert_eq!(backend.append_calls(), 4, "a probe and 256 + 256 + 88");
}

fn assistant_line(message: &str, uuid: &str, block: &str) -> String {
    format!(
        r#"{{"type":"assistant","uuid":"{uuid}","sessionId":"{ID}","message":{{"id":"{message}","role":"assistant","content":[{block}],"usage":{{"input_tokens":5,"output_tokens":2}}}}}}"#
    )
}

#[tokio::test]
async fn usage_ships_once_per_model_response() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_file(dir.path(), dir.path());
    let text = r#"{"type":"text","text":"a"}"#;
    let call = r#"{"type":"tool_use","id":"toolu_1","name":"Bash","input":{}}"#;
    std::fs::write(
        &path,
        format!(
            "{}\n{}\n",
            assistant_line("msg_1", "u1", text),
            assistant_line("msg_1", "u2", call)
        ),
    )
    .unwrap();
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    recorder(backend.clone(), dir.path(), dir.path())
        .0
        .flush()
        .await
        .unwrap();

    // A restarted recorder takes the held tip's item id.
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    writeln!(file, "{}", assistant_line("msg_1", "u3", text)).unwrap();
    writeln!(file, "{}", assistant_line("msg_2", "u4", text)).unwrap();
    recorder(backend.clone(), dir.path(), dir.path())
        .0
        .flush()
        .await
        .unwrap();

    let usage: Vec<bool> = backend
        .events_on(ID)
        .iter()
        .map(|e| e.content.get("usage").is_some())
        .collect();
    assert_eq!(usage, [true, false, false, true]);
}

#[tokio::test]
async fn a_subagent_file_becomes_a_session_linked_to_its_call() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_file(dir.path(), dir.path());
    let call = r#"{"type":"tool_use","id":"toolu_1","name":"Agent","input":{}}"#;
    std::fs::write(&path, format!("{}\n", assistant_line("msg_1", "u1", call))).unwrap();
    let subagents = path.with_extension("").join("subagents");
    std::fs::create_dir_all(&subagents).unwrap();
    std::fs::write(
        subagents.join("agent-a1.meta.json"),
        r#"{"agentType":"general-purpose","toolUseId":"toolu_1"}"#,
    )
    .unwrap();
    std::fs::write(
        subagents.join("agent-a1.jsonl"),
        format!(
            "{}\n",
            r#"{"type":"user","isSidechain":true,"agentId":"a1","message":{"role":"user","content":"go"}}"#
        ),
    )
    .unwrap();
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let (main, _) = recorder(backend.clone(), dir.path(), dir.path());

    let live = LiveRecorder::spawn(main);
    live.flush().await;

    let subagent = format!("{ID}:agent-a1");
    assert_eq!(backend.parent_of(&subagent), Some((ID.to_string(), 0)));
    assert_eq!(backend.events_on(&subagent).len(), 1);
}

#[tokio::test]
async fn usage_survives_a_failed_append() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_file(dir.path(), dir.path());
    let text = r#"{"type":"text","text":"a"}"#;
    std::fs::write(&path, format!("{}\n", assistant_line("msg_1", "u1", text))).unwrap();
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let (mut recorder, _) = recorder(backend.clone(), dir.path(), dir.path());

    backend.fail_appends(1);
    assert!(recorder.flush().await.is_err());
    recorder.flush().await.unwrap();

    let events = backend.events_on(ID);
    assert_eq!(events.len(), 1);
    assert!(events[0].content.get("usage").is_some());
}

#[tokio::test]
async fn a_subagent_whose_call_is_not_recorded_leaves_recording_running() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_file(dir.path(), dir.path());
    let text = r#"{"type":"text","text":"a"}"#;
    std::fs::write(&path, format!("{}\n", assistant_line("msg_1", "u1", text))).unwrap();
    let subagents = path.with_extension("").join("subagents");
    std::fs::create_dir_all(&subagents).unwrap();
    std::fs::write(
        subagents.join("agent-a1.meta.json"),
        r#"{"toolUseId":"toolu_unrecorded"}"#,
    )
    .unwrap();
    std::fs::write(
        subagents.join("agent-a1.jsonl"),
        "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"go\"}}\n",
    )
    .unwrap();
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let (main, _) = recorder(backend.clone(), dir.path(), dir.path());

    let live = LiveRecorder::spawn(main);
    live.flush().await;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    writeln!(file, "{}", assistant_line("msg_2", "u2", text)).unwrap();
    live.flush().await;

    assert_eq!(backend.events_on(ID).len(), 2);
    assert!(backend.events_on(&format!("{ID}:agent-a1")).is_empty());
}
