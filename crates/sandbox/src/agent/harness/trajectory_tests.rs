use super::*;

const CLAUDE_THINKING: &str =
    include_str!("../../../tests/fixtures/trajectory/claude-thinking-tools.jsonl");
const CLAUDE_COMPACTED: &str =
    include_str!("../../../tests/fixtures/trajectory/claude-killed-compacted.jsonl");
const CODEX: &str =
    include_str!("../../../tests/fixtures/trajectory/codex-reasoning-killed-compacted.jsonl");

fn read_all(format: &dyn SessionFormat, body: &str) -> Vec<RecordedLine> {
    body.lines().map(|line| format.read_line(line)).collect()
}

fn blocks_of<'a>(
    lines: &'a [RecordedLine],
    pick: impl Fn(&CanonicalContentBlock) -> bool + 'a,
) -> impl Iterator<Item = &'a CanonicalContentBlock> + 'a {
    lines
        .iter()
        .flat_map(|l| l.content.blocks.iter())
        .filter(move |b| pick(b))
}

#[test]
fn content_of_bookkeeping_lines_is_empty() {
    let claude = read_all(&ClaudeSessionFormat, CLAUDE_THINKING);
    let codex = read_all(&CodexSessionFormat, CODEX);
    let bookkeeping: Vec<&RecordedLine> = claude
        .iter()
        .filter(|l| l.kind == "cost-state")
        .chain(codex.iter().filter(|l| l.kind == "event_msg/token_count"))
        .collect();

    assert!(bookkeeping.iter().any(|l| l.kind == "cost-state"));
    assert!(
        bookkeeping
            .iter()
            .any(|l| l.kind == "event_msg/token_count")
    );
    for line in bookkeeping {
        assert_eq!(line.content.to_json(), r#"{"role":null,"blocks":[]}"#);
    }
}

#[test]
fn codex_projects_each_item_once() {
    let lines = read_all(&CodexSessionFormat, CODEX);
    let calls = lines
        .iter()
        .filter(|l| {
            l.kind == "response_item/function_call" || l.kind == "response_item/custom_tool_call"
        })
        .count();
    let tool_uses = blocks_of(&lines, |b| {
        matches!(b, CanonicalContentBlock::ToolUse { .. })
    });

    assert!(calls > 0, "the fixture carries tool calls");
    assert_eq!(tool_uses.count(), calls, "one ToolUse per call");
    for line in lines
        .iter()
        .filter(|l| !l.kind.starts_with("response_item/"))
    {
        assert!(line.content.blocks.is_empty(), "{} got blocks", line.kind);
    }
}

#[test]
fn an_unparseable_line_is_recorded() {
    for format in [
        &ClaudeSessionFormat as &dyn SessionFormat,
        &CodexSessionFormat,
    ] {
        let line = format.read_line("{\"type\":\"assistant\",\"message\":");
        assert_eq!(line.kind, "unparsed");
        assert_eq!(line.content, EventContent::default());
        assert_eq!(format.retarget("not json", "new"), "not json");
    }
}

#[test]
fn claude_thinking_keeps_its_signature() {
    let lines = read_all(&ClaudeSessionFormat, CLAUDE_THINKING);
    let thinking: Vec<_> = blocks_of(&lines, |b| {
        matches!(b, CanonicalContentBlock::Thinking { .. })
    })
    .collect();
    assert!(!thinking.is_empty());
    for block in thinking {
        let CanonicalContentBlock::Thinking { signature, .. } = block else {
            unreachable!()
        };
        assert!(!signature.is_empty(), "signed thinking keeps its signature");
    }
}

#[test]
fn codex_reasoning_projects_its_summary_without_a_signature() {
    let lines = read_all(&CodexSessionFormat, CODEX);
    let reasoning: Vec<_> = lines
        .iter()
        .filter(|l| l.kind == "response_item/reasoning")
        .collect();
    assert!(!reasoning.is_empty());
    let summaries: Vec<_> = reasoning
        .iter()
        .flat_map(|l| l.content.blocks.iter())
        .collect();
    assert!(!summaries.is_empty());
    for block in summaries {
        let CanonicalContentBlock::Thinking {
            thinking,
            signature,
        } = block
        else {
            panic!("reasoning projects to Thinking, got {block:?}");
        };
        assert!(!thinking.is_empty());
        assert!(signature.is_empty());
    }
    for line in reasoning {
        assert!(!line.content.to_json().contains("encrypted_content"));
    }
}

#[test]
fn item_and_turn_ids_are_read() {
    let claude = read_all(&ClaudeSessionFormat, CLAUDE_THINKING);
    let assistant = claude.iter().find(|l| l.kind == "assistant").unwrap();
    assert!(
        assistant
            .harness_item_id
            .as_deref()
            .unwrap()
            .starts_with("msg_")
    );

    let codex = read_all(&CodexSessionFormat, CODEX);
    let complete = codex
        .iter()
        .find(|l| l.kind == "event_msg/task_complete")
        .unwrap();
    assert!(complete.harness_turn_id.is_some());
    let message = codex
        .iter()
        .find(|l| l.kind == "response_item/message")
        .unwrap();
    assert!(
        message
            .harness_item_id
            .as_deref()
            .unwrap()
            .starts_with("msg_")
    );
}

/// Every value `retarget` owns, found by parsing.
fn id_values(format: &dyn SessionFormat, line: &str) -> Vec<String> {
    let value: Value = serde_json::from_str(line).unwrap();
    if format.harness() == "claude-code" {
        return str_at(&value, &["sessionId"])
            .map(str::to_string)
            .into_iter()
            .collect();
    }
    let mut fields = Vec::new();
    collect_thread_ids(&value, &mut fields);
    let mut ids: Vec<String> = fields.into_iter().map(|(_, v)| v).collect();
    if str_at(&value, &["type"]) == Some("session_meta") {
        ids.extend(str_at(&value, &["payload", "id"]).map(str::to_string));
    }
    ids
}

#[test]
fn retarget_rewrites_only_the_session_id_fields() {
    let new_id = "0199aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee";
    for (format, body) in [
        (&ClaudeSessionFormat as &dyn SessionFormat, CLAUDE_THINKING),
        (&ClaudeSessionFormat, CLAUDE_COMPACTED),
        (&CodexSessionFormat, CODEX),
    ] {
        let mut rewritten = 0;
        for line in body.lines() {
            let out = format.retarget(line, new_id);
            let ids = id_values(format, &out);
            assert!(ids.iter().all(|id| id == new_id), "{ids:?} in {out}");
            rewritten += usize::from(!ids.is_empty());

            // Only the id fields moved: putting the old ids back gives the line.
            let mut restored = out.clone();
            for (old, new) in id_values(format, line).iter().zip(&ids) {
                restored = restored.replacen(&format!("\"{new}\""), &format!("\"{old}\""), 1);
            }
            assert_eq!(restored, line);
        }
        assert!(rewritten > 0);
    }
}

#[test]
fn claude_session_path_dashes_every_non_alphanumeric() {
    let path = ClaudeSessionFormat.session_path(
        Path::new("/home/c"),
        Path::new("/nonexistent/dir.with_dots"),
        "abc",
        "",
    );
    assert_eq!(
        path,
        Path::new("/home/c/projects/-nonexistent-dir-with-dots/abc.jsonl")
    );
}

#[test]
fn codex_session_path_is_dated_from_the_first_line() {
    let first = CODEX.lines().next().unwrap();
    let path = CodexSessionFormat.session_path(Path::new("/h"), Path::new("/w"), "tid", first);
    assert_eq!(
        path,
        Path::new("/h/sessions/2026/09/25/rollout-2026-09-25T15-16-12-tid.jsonl")
    );
}

#[test]
fn codex_locate_finds_a_rollout_by_id() {
    let home = tempfile::tempdir().unwrap();
    let path = CodexSessionFormat.session_path(home.path(), Path::new("/w"), "tid", "");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "").unwrap();
    assert_eq!(
        CodexSessionFormat.locate(home.path(), Path::new("/w"), "tid"),
        Some(path)
    );
    assert_eq!(
        CodexSessionFormat.locate(home.path(), Path::new("/w"), "other"),
        None
    );
}

use crate::backend_client::BackendClient;
use crate::trajectory_tail::memory::MemoryTrajectoryBackend;
use sandbox_proto::{AppendTrajectoryRequest, TrajectoryLine};

const SESSION: &str = "0199aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee";

/// A backend holding the first five lines of the Claude fixture under
/// `SESSION`, and those lines.
async fn five_line_session() -> (MemoryTrajectoryBackend, Vec<String>) {
    let backend = MemoryTrajectoryBackend::default();
    let lines: Vec<String> = CLAUDE_THINKING
        .lines()
        .take(5)
        .map(|l| ClaudeSessionFormat.retarget(l, SESSION))
        .collect();
    let wire = lines
        .iter()
        .map(|l| {
            let read = ClaudeSessionFormat.read_line(l);
            TrajectoryLine {
                kind: read.kind,
                harness_item_id: read.harness_item_id,
                harness_turn_id: read.harness_turn_id,
                content_json: read.content.to_json(),
                payload: l.clone(),
            }
        })
        .collect();
    let reply = backend
        .append_trajectory(AppendTrajectoryRequest {
            task_id: "task".into(),
            harness: "claude-code".into(),
            harness_session_id: SESSION.into(),
            first_seq: 0,
            lines: wire,
            turn_number: None,
        })
        .await
        .unwrap();
    assert_eq!(reply.last_seq, 4);
    (backend, lines)
}

async fn write(backend: &MemoryTrajectoryBackend, home: &Path) -> Option<Written> {
    write_session(
        backend,
        &(Arc::new(ClaudeSessionFormat) as Arc<dyn SessionFormat>),
        "task",
        home,
        Path::new("/w"),
        WriteTarget::Resume(SESSION.into()),
    )
    .await
}

#[tokio::test]
async fn a_stale_archive_file_is_replaced() {
    let (backend, lines) = five_line_session().await;
    let home = tempfile::tempdir().unwrap();
    let path = ClaudeSessionFormat.session_path(home.path(), Path::new("/w"), SESSION, "");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();

    std::fs::write(&path, format!("{}\n", lines[..3].join("\n"))).unwrap();
    let written = write(&backend, home.path()).await.unwrap();
    assert_eq!(written.line_count, 5);
    assert!(written.rewritten);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        format!("{}\n", lines.join("\n"))
    );

    let kept = format!("{}\n{{\"type\":\"sixth\"}}\n", lines.join("\n"));
    std::fs::write(&path, &kept).unwrap();
    let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
    assert!(!write(&backend, home.path()).await.unwrap().rewritten);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), kept);
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), mtime);
}

#[tokio::test]
async fn falls_back_to_the_archive_on_a_gap() {
    let (backend, _) = five_line_session().await;
    backend.lose(SESSION, 3);
    let home = tempfile::tempdir().unwrap();

    assert_eq!(write(&backend, home.path()).await, None);
    assert!(
        ClaudeSessionFormat
            .locate(home.path(), Path::new("/w"), SESSION)
            .is_none(),
        "nothing is written"
    );
}

#[test]
fn claude_latest_is_the_newest_transcript() {
    let home = tempfile::tempdir().unwrap();
    let cwd = Path::new("/w");
    assert_eq!(ClaudeSessionFormat.latest(home.path(), cwd), None);

    let dir = ClaudeSessionFormat
        .session_path(home.path(), cwd, "x", "")
        .parent()
        .unwrap()
        .to_path_buf();
    std::fs::create_dir_all(&dir).unwrap();
    let base = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    for (name, age) in [("old.jsonl", 60), ("new.jsonl", 10), ("newest.txt", 0)] {
        let file = std::fs::File::create(dir.join(name)).unwrap();
        file.set_modified(base - std::time::Duration::from_secs(age))
            .unwrap();
    }
    assert_eq!(
        ClaudeSessionFormat.latest(home.path(), cwd).as_deref(),
        Some("new")
    );
}
