use std::fmt::Write as _;
use std::io::Write as _;

use super::*;
use crate::agent::harness::trajectory::{ClaudeSessionFormat, CodexSessionFormat};
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

/// Claude, except that a line containing `"derived"` is another view of the
/// call open when it was written, as a Codex command line is.
struct DerivedLines;

impl SessionFormat for DerivedLines {
    fn harness(&self) -> &'static str {
        ClaudeSessionFormat.harness()
    }
    fn session_path(
        &self,
        home: &std::path::Path,
        cwd: &std::path::Path,
        id: &str,
        first_line: &str,
    ) -> PathBuf {
        ClaudeSessionFormat.session_path(home, cwd, id, first_line)
    }
    fn locate(&self, home: &std::path::Path, cwd: &std::path::Path, id: &str) -> Option<PathBuf> {
        ClaudeSessionFormat.locate(home, cwd, id)
    }
    fn read_line(&self, line: &str) -> crate::agent::harness::trajectory::RecordedLine {
        let mut read = ClaudeSessionFormat.read_line(line);
        read.derived = line.contains("\"derived\"");
        read
    }
    fn retarget(&self, payload: &str, id: &str) -> String {
        ClaudeSessionFormat.retarget(payload, id)
    }
    fn latest(&self, home: &std::path::Path, cwd: &std::path::Path) -> Option<String> {
        ClaudeSessionFormat.latest(home, cwd)
    }
    fn render(
        &self,
        items: &[crate::agent::harness::trajectory::EventContent],
        ctx: &crate::agent::harness::trajectory::RenderContext<'_>,
    ) -> Vec<String> {
        ClaudeSessionFormat.render(items, ctx)
    }
}

/// A restarted recorder rebuilds the open calls from the lines it skips, so a
/// derived line after the restart still names its call.
#[tokio::test]
async fn a_restarted_recorder_still_names_the_open_call() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_file(dir.path(), dir.path());
    let call = r#"{"type":"tool_use","id":"call_exec","name":"exec","input":{}}"#;
    let derived = r#"{"type":"text","text":"derived"}"#;
    std::fs::write(&path, format!("{}\n", assistant_line("msg_1", "u1", call))).unwrap();
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let start = |backend: &Arc<MemoryTrajectoryBackend>| {
        let target = RecordTarget {
            task_id: "task".into(),
            harness_session_id: ID.into(),
            home: dir.path().to_path_buf(),
            cwd: dir.path().to_path_buf(),
            subagent: None,
        };
        SessionRecorder::start(
            Arc::new(DerivedLines),
            TrajectoryRecording::new(backend.clone()),
            target,
            None,
        )
    };
    start(&backend).flush().await.unwrap();

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    writeln!(file, "{}", assistant_line("exec-1", "u2", derived)).unwrap();
    start(&backend).flush().await.unwrap();

    let events = backend.events_on(ID);
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].content["derived_from"], "call_exec");
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

/// Writes the subagent `agent` spawned by `tool_use_id`, its lines `lines`.
fn subagent_file(session: &std::path::Path, agent: &str, tool_use_id: &str, lines: &[String]) {
    let subagents = session.with_extension("").join("subagents");
    std::fs::create_dir_all(&subagents).unwrap();
    std::fs::write(
        subagents.join(format!("agent-{agent}.meta.json")),
        format!(r#"{{"toolUseId":"{tool_use_id}"}}"#),
    )
    .unwrap();
    let body: String = lines.iter().flat_map(|l| [l.as_str(), "\n"]).collect();
    std::fs::write(subagents.join(format!("agent-{agent}.jsonl")), body).unwrap();
}

#[tokio::test]
async fn a_subagent_a_subagent_spawned_links_to_the_call_in_its_spawner() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_file(dir.path(), dir.path());
    let call =
        |id: &str| format!(r#"{{"type":"tool_use","id":"{id}","name":"Agent","input":{{}}}}"#);
    std::fs::write(
        &path,
        format!("{}\n", assistant_line("msg_1", "u1", &call("toolu_1"))),
    )
    .unwrap();
    subagent_file(
        &path,
        "a1",
        "toolu_1",
        &[assistant_line("msg_2", "u2", &call("toolu_2"))],
    );
    let go = r#"{"type":"user","message":{"role":"user","content":"go"}}"#.to_string();
    subagent_file(&path, "b1", "toolu_2", &[go]);
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let (main, recording) = recorder(backend.clone(), dir.path(), dir.path());
    let mut files = ClaudeSessionFormat.subagents(&path);
    files.sort_by(|a, b| b.harness_session_id.cmp(&a.harness_session_id));
    let subagents = files
        .into_iter()
        .map(|file| {
            let target = main.target.subagent(file);
            SessionRecorder::start(
                Arc::new(ClaudeSessionFormat),
                Arc::clone(&recording),
                target,
                None,
            )
        })
        .collect();
    let mut sessions = LiveSessions {
        main,
        subagents,
        warned: Vec::new(),
        watcher: None,
    };

    // `b1` is flushed before the `a1` that spawned it.
    sessions.flush(&mpsc::channel(1).0, true).await;

    let nested = format!("{ID}:agent-b1");
    assert_eq!(
        backend.parent_of(&nested),
        Some((format!("{ID}:agent-a1"), 0))
    );
    assert_eq!(backend.events_on(&nested).len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_subagent_whose_call_is_not_recorded_is_asked_after_a_growing_wait() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_file(dir.path(), dir.path());
    let go = r#"{"type":"user","message":{"role":"user","content":"go"}}"#.to_string();
    subagent_file(&path, "a1", "toolu_1", &[go]);
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let (main, recording) = recorder(backend.clone(), dir.path(), dir.path());
    let file = ClaudeSessionFormat.subagents(&path).pop().unwrap();
    let mut subagent = SessionRecorder::start(
        Arc::new(ClaudeSessionFormat),
        recording,
        main.target.subagent(file),
        None,
    );
    let asks = |n| {
        let backend = Arc::clone(&backend);
        move || assert_eq!(backend.append_calls(), n)
    };

    assert!(subagent.flush().await.is_err());
    asks(1)();
    subagent.flush().await.unwrap();
    tokio::time::advance(Duration::from_millis(999)).await;
    subagent.flush().await.unwrap();
    asks(1)();
    tokio::time::advance(Duration::from_millis(1)).await;
    assert!(subagent.flush().await.is_err());
    asks(2)();
    tokio::time::advance(Duration::from_millis(1999)).await;
    subagent.flush().await.unwrap();
    asks(2)();

    let call = r#"{"type":"tool_use","id":"toolu_1","name":"Agent","input":{}}"#;
    std::fs::write(&path, format!("{}\n", assistant_line("msg_1", "u1", call))).unwrap();
    let (mut main, _) = recorder(backend.clone(), dir.path(), dir.path());
    main.flush().await.unwrap();
    assert_eq!(
        subagent.flush_all().await.unwrap(),
        0,
        "a release flush asks at once"
    );
    assert_eq!(backend.events_on(&format!("{ID}:agent-a1")).len(), 1);
}

fn turn(backend: &Arc<MemoryTrajectoryBackend>, home: &std::path::Path) -> TurnTrajectory {
    TurnTrajectory {
        recording: TrajectoryRecording::new(Arc::clone(backend) as Arc<dyn BackendClient>),
        format: Arc::new(ClaudeSessionFormat),
        task_id: "task".into(),
        home: home.to_path_buf(),
        cwd: home.to_path_buf(),
        written: None,
        live: None,
    }
}

fn fork_input(switch_harness: bool) -> Input {
    let mut input = Input::text("in-1", "hi");
    input.start_fresh = true;
    input.switch_harness = switch_harness;
    input.fork = Some(crate::agent::types::ForkSource {
        source_session_id: "thread-1".into(),
        source_message_uuid: None,
        source_agent_turn_id: Some("turn-1".into()),
    });
    input
}

#[tokio::test]
async fn an_unreadable_cross_harness_fork_fails_the_turn() {
    let home = tempfile::tempdir().unwrap();
    let backend = Arc::new(MemoryTrajectoryBackend::default());

    let refused = turn(&backend, home.path())
        .write_out(&fork_input(true))
        .await;

    let err = refused.expect_err("the fork's history cannot reach this harness");
    assert_eq!(err.class, ErrorClass::UnavailableSession);
}

#[tokio::test]
async fn an_unreadable_same_harness_fork_falls_back_to_the_cli() {
    let home = tempfile::tempdir().unwrap();
    let backend = Arc::new(MemoryTrajectoryBackend::default());

    let fallback = turn(&backend, home.path())
        .write_out(&fork_input(false))
        .await
        .expect("the CLI forks its own transcript");

    assert_eq!(fallback, None);
}

const CODEX_ID: &str = "01a0de79-4dff-72c0-a0de-c819e502d676";
/// Line 43 is Codex's compaction summary reply, restated by the `compacted`
/// line 46; lines 44 and 45 carry no role.
const CODEX_COMPACTED: &str = include_str!("../tests/fixtures/transcode/codex.jsonl");

/// A Codex recorder over a rollout holding the fixture's lines before `upto`,
/// all of them recorded.
async fn codex_recorder(
    backend: &Arc<MemoryTrajectoryBackend>,
    home: &std::path::Path,
    upto: usize,
) -> (SessionRecorder, PathBuf) {
    let lines: Vec<&str> = CODEX_COMPACTED.lines().collect();
    let path = CodexSessionFormat.session_path(home, home, CODEX_ID, lines[0]);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, format!("{}\n", lines[..upto].join("\n"))).unwrap();
    let backend: Arc<dyn BackendClient> = Arc::clone(backend) as Arc<dyn BackendClient>;
    let mut recorder = SessionRecorder::start(
        Arc::new(CodexSessionFormat),
        TrajectoryRecording::new(backend),
        RecordTarget {
            task_id: "task".into(),
            harness_session_id: CODEX_ID.into(),
            home: home.to_path_buf(),
            cwd: home.to_path_buf(),
            subagent: None,
        },
        None,
    );
    assert_eq!(recorder.flush().await.unwrap(), upto as i64 - 1);
    (recorder, path)
}

fn append_fixture(path: &std::path::Path, range: std::ops::Range<usize>) {
    let lines: Vec<&str> = CODEX_COMPACTED.lines().collect();
    let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    for line in &lines[range] {
        writeln!(file, "{line}").unwrap();
    }
}

#[tokio::test]
async fn a_codex_reply_ships_in_one_append_with_the_line_that_supersedes_it() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let (mut recorder, path) = codex_recorder(&backend, dir.path(), 43).await;

    append_fixture(&path, 43..46);
    assert_eq!(
        recorder.flush().await.unwrap(),
        42,
        "the reply and the role-less lines wait"
    );

    let appends = backend.append_calls();
    append_fixture(&path, 46..48);
    assert_eq!(recorder.flush().await.unwrap(), 47);
    assert_eq!(backend.append_calls(), appends + 1);
    let events = backend.events_on(CODEX_ID);
    assert_eq!(
        events[46].content["supersedes"].as_str(),
        events[43].harness_item_id.as_deref()
    );
}

#[tokio::test]
async fn a_held_codex_reply_ships_at_turn_end() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let (mut recorder, path) = codex_recorder(&backend, dir.path(), 43).await;

    append_fixture(&path, 43..44);
    assert_eq!(recorder.flush().await.unwrap(), 42);
    assert_eq!(recorder.flush_all().await.unwrap(), 43);
}

#[tokio::test(start_paused = true)]
async fn a_held_codex_reply_ships_once_it_has_waited() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    let (mut recorder, path) = codex_recorder(&backend, dir.path(), 43).await;

    append_fixture(&path, 43..45);
    assert_eq!(recorder.flush().await.unwrap(), 42);
    tokio::time::advance(Duration::from_millis(499)).await;
    assert_eq!(recorder.flush().await.unwrap(), 42);
    tokio::time::advance(Duration::from_millis(1)).await;
    assert_eq!(recorder.flush().await.unwrap(), 44);
}

#[tokio::test]
async fn an_append_over_the_decode_limit_stops_the_recorder_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let path = session_file(dir.path(), dir.path());
    let large = format!("{{\"data\":\"{}\"}}", "J".repeat(64 * 1024));
    std::fs::write(
        &path,
        format!("{{\"type\":\"user\"}}\n{large}\n{{\"n\":2}}\n"),
    )
    .unwrap();
    let backend = Arc::new(MemoryTrajectoryBackend::default());
    backend.limit_appends(32 * 1024);
    let (mut recorder, _) = recorder(backend.clone(), dir.path(), dir.path());

    let refused = recorder.flush().await.unwrap_err();
    assert!(
        matches!(&refused, RecordError::Rpc(status) if status.code() == tonic::Code::OutOfRange)
    );
    assert!(matches!(recorder.flush().await, Err(RecordError::Off)));
    let published = backend.published();
    assert_eq!(published.len(), 1);
    assert_eq!(published[0].0, "recording_stopped");
    assert!(published[0].1.contains("too large"), "{}", published[0].1);
}
