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
fn bookkeeping_lines_carry_no_blocks() {
    let claude = read_all(&ClaudeSessionFormat, CLAUDE_THINKING);
    let codex = read_all(&CodexSessionFormat, CODEX);

    let cost: Vec<&RecordedLine> = claude.iter().filter(|l| l.kind == "cost-state").collect();
    assert!(!cost.is_empty());
    for line in cost {
        assert_eq!(
            line.content.to_json(),
            r#"{"role":null,"blocks":[],"injected":false,"compaction":false}"#
        );
    }
    let counts: Vec<&RecordedLine> = codex
        .iter()
        .filter(|l| l.kind == "event_msg/token_count")
        .collect();
    assert!(!counts.is_empty());
    assert!(counts.iter().all(|l| l.content.blocks.is_empty()));
}

#[test]
fn usage_is_read_net_of_cached_input() {
    let claude = read_all(&ClaudeSessionFormat, CLAUDE_THINKING);
    let usage: Vec<&Usage> = claude
        .iter()
        .filter_map(|l| l.content.usage.as_ref())
        .collect();
    assert!(usage.iter().any(|u| u.cache_read_tokens > 0));
    assert!(
        claude
            .iter()
            .filter(|l| l.content.usage.is_some())
            .all(|l| l.content.role == Some(Role::Assistant))
    );

    let line = r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"cached_input_tokens":60,"output_tokens":7}}}}"#;
    let usage = CodexSessionFormat.read_line(line).content.usage.unwrap();
    assert_eq!(
        (
            usage.input_tokens,
            usage.cache_read_tokens,
            usage.output_tokens
        ),
        (40, 60, 7)
    );
    let without = r#"{"type":"event_msg","payload":{"type":"token_count","info":null}}"#;
    assert!(
        CodexSessionFormat
            .read_line(without)
            .content
            .usage
            .is_none()
    );
    let codex = read_all(&CodexSessionFormat, CODEX);
    assert!(codex.iter().any(|l| l.content.usage.is_some()));
}

#[test]
fn a_compaction_summary_is_marked_and_not_injected() {
    for (format, body) in [
        (&ClaudeSessionFormat as &dyn SessionFormat, CLAUDE_COMPACTED),
        (&CodexSessionFormat, CODEX),
    ] {
        let lines = read_all(format, body);
        let summaries: Vec<&RecordedLine> = lines.iter().filter(|l| l.content.compaction).collect();
        assert_eq!(summaries.len(), 1, "{}", format.harness());
        let summary = summaries[0];
        assert!(!summary.content.injected);
        assert_eq!(summary.content.role, Some(Role::User));
        assert!(matches!(
            summary.content.blocks.as_slice(),
            [CanonicalContentBlock::Text { text }] if !text.is_empty()
        ));
    }
}

#[test]
fn harness_injected_lines_are_marked() {
    let claude = read_all(&ClaudeSessionFormat, CLAUDE_COMPACTED);
    let raw: Vec<&str> = CLAUDE_COMPACTED.lines().collect();
    for (line, read) in raw.iter().zip(&claude) {
        if line.contains(r#""isMeta":true"#) {
            assert!(read.content.injected, "{line}");
        }
    }
    let typed = claude
        .iter()
        .filter(|l| l.content.role == Some(Role::User) && !l.content.injected)
        .filter(|l| {
            l.content
                .blocks
                .iter()
                .any(|b| matches!(b, CanonicalContentBlock::Text { .. }))
        })
        .count();
    assert!(typed > 0, "a typed prompt is not injected");

    let codex = read_all(&CodexSessionFormat, CODEX);
    let raw: Vec<&str> = CODEX.lines().collect();
    for (line, read) in raw.iter().zip(&codex) {
        if !line.contains(r#""type":"response_item""#) {
            continue;
        }
        let developer = line.contains(r#""role":"developer""#);
        let context = line.contains(r#""role":"user""#) && line.contains("<environment_context>");
        if (developer || context) && !read.content.blocks.is_empty() {
            assert!(read.content.injected, "{line}");
        }
    }
    let prompt = r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}}"#;
    assert!(!CodexSessionFormat.read_line(prompt).content.injected);
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

    let commands = lines.iter().filter(|l| l.derived).count();

    assert!(calls > 0, "the fixture carries tool calls");
    assert_eq!(commands, 1, "the fixture's exec call ran one command");
    assert_eq!(
        tool_uses.count(),
        calls + commands,
        "one ToolUse per call and per command it ran"
    );
    // A compaction line holds its summary.
    for line in lines
        .iter()
        .filter(|l| !l.kind.starts_with("response_item/") && l.kind != "compacted" && !l.derived)
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
            subagent_parent: None,
            own_from_seq: None,
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
        None,
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

const CLAUDE_RECORDED: &str = include_str!("../../../tests/fixtures/transcode/claude.jsonl");
const CODEX_RECORDED: &str = include_str!("../../../tests/fixtures/transcode/codex.jsonl");

/// The items the backend's `transcode_items` selects from these lines: own
/// lines with content, from the last compaction on.
fn transcode_items(format: &dyn SessionFormat, lines: &[&str]) -> Vec<EventContent> {
    let mut items = Vec::new();
    let mut calls = LineContext::default();
    for line in lines {
        let mut read = format.read_line(line);
        calls.observe(&mut read);
        let content = read.content;
        if content.compaction {
            items.clear();
        }
        if !content.injected
            && content.derived_from.is_none()
            && content.role.is_some()
            && !content.blocks.is_empty()
        {
            items.push(content);
        }
    }
    items
}

fn render_as(
    to: &dyn SessionFormat,
    from: &dyn SessionFormat,
    recorded: &str,
    take: usize,
) -> Vec<Value> {
    let lines: Vec<&str> = recorded.lines().take(take).collect();
    let items = transcode_items(from, &lines);
    let ctx = RenderContext {
        harness_session_id: "{{SESSION_ID}}",
        cwd: Path::new("/workspace"),
        model: Some("claude-haiku-4-5-20251001"),
        prior: &[],
        timestamp: "2026-09-26T00:00:00.000Z",
    };
    to.render(&items, &ctx)
        .iter()
        .map(|line| serde_json::from_str(&line.replace("/workspace", "{{CWD}}")).unwrap())
        .collect()
}

fn fixture(body: &str) -> Vec<Value> {
    body.lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn claude_renders_as_codex() {
    let rendered = render_as(
        &CodexSessionFormat,
        &ClaudeSessionFormat,
        CLAUDE_RECORDED,
        39,
    );
    assert_eq!(
        rendered,
        fixture(include_str!(
            "../../../tests/fixtures/transcode/codex-from-claude.jsonl"
        ))
    );
}

#[test]
fn codex_renders_as_claude() {
    let rendered = render_as(
        &ClaudeSessionFormat,
        &CodexSessionFormat,
        CODEX_RECORDED,
        38,
    );
    assert_eq!(
        rendered,
        fixture(include_str!(
            "../../../tests/fixtures/transcode/claude-from-codex.jsonl"
        ))
    );
}

#[test]
fn a_compacted_session_renders_from_its_summary() {
    let codex = render_as(
        &CodexSessionFormat,
        &ClaudeSessionFormat,
        CLAUDE_RECORDED,
        usize::MAX,
    );
    assert_eq!(
        codex,
        fixture(include_str!(
            "../../../tests/fixtures/transcode/codex-from-claude-compacted.jsonl"
        ))
    );
    let claude = render_as(
        &ClaudeSessionFormat,
        &CodexSessionFormat,
        CODEX_RECORDED,
        usize::MAX,
    );
    assert_eq!(
        claude,
        fixture(include_str!(
            "../../../tests/fixtures/transcode/claude-from-codex-compacted.jsonl"
        ))
    );
}

#[test]
fn rendering_after_lines_chains_onto_the_last_with_a_uuid() {
    let items = transcode_items(
        &CodexSessionFormat,
        &CODEX_RECORDED.lines().take(38).collect::<Vec<_>>(),
    );
    let prior = [
        r#"{"type":"assistant","uuid":"11111111-2222-3333-4444-555555555555"}"#,
        r#"{"type":"last-prompt"}"#,
    ];
    let ctx = |prior| RenderContext {
        harness_session_id: "s",
        cwd: Path::new("/w"),
        model: None,
        prior,
        timestamp: "t",
    };
    let claude = ClaudeSessionFormat.render(&items, &ctx(&prior));
    let first: Value = serde_json::from_str(&claude[0]).unwrap();
    assert_eq!(first["parentUuid"], "11111111-2222-3333-4444-555555555555");

    let codex = CodexSessionFormat.render(&items, &ctx(&prior));
    assert!(codex.iter().all(|l| !l.contains("session_meta")));
    assert!(CodexSessionFormat.render(&items, &ctx(&[]))[0].contains("session_meta"));
}

#[test]
fn an_unanswered_call_is_not_rendered() {
    let items = vec![
        EventContent {
            role: Some(Role::Assistant),
            blocks: vec![CanonicalContentBlock::ToolUse {
                id: "c1".into(),
                name: "Bash".into(),
                input: serde_json::json!({"command": "ls"}),
            }],
            ..EventContent::default()
        },
        EventContent {
            role: Some(Role::User),
            blocks: vec![CanonicalContentBlock::ToolResult {
                tool_use_id: "gone".into(),
                content: None,
                is_error: None,
            }],
            ..EventContent::default()
        },
    ];
    let ctx = RenderContext {
        harness_session_id: "s",
        cwd: Path::new("/w"),
        model: None,
        prior: &["{}"],
        timestamp: "t",
    };
    assert!(CodexSessionFormat.render(&items, &ctx).is_empty());
    assert!(ClaudeSessionFormat.render(&items, &ctx).is_empty());
}

#[test]
fn content_json_reads_back() {
    let line = CodexSessionFormat.read_line(CODEX_RECORDED.lines().nth(5).unwrap());
    let back: EventContent = serde_json::from_str(&line.content.to_json()).unwrap();
    assert_eq!(back, line.content);
}

/// A code-mode `exec` call's command is its own `codex.shell` call and result,
/// shown under the `exec` call that was open when Codex ran it.
#[test]
fn a_codex_command_is_a_view_of_the_exec_call_that_ran_it() {
    let mut calls = LineContext::default();
    let mut lines = read_all(&CodexSessionFormat, CODEX);
    for line in &mut lines {
        calls.observe(line);
    }
    let command = lines.iter().find(|l| l.derived).unwrap();

    assert_eq!(
        command.content.derived_from.as_deref(),
        Some("call_7o2cP988tSqkhfkN2WCacs8H")
    );
    assert_eq!(
        command.harness_item_id.as_deref(),
        Some("exec-f23c77a5-f8e0-4637-90b7-67f1074dc9f0")
    );
    let [
        CanonicalContentBlock::ToolUse { id, name, input },
        CanonicalContentBlock::ToolResult {
            tool_use_id,
            is_error,
            ..
        },
    ] = command.content.blocks.as_slice()
    else {
        panic!("a call and its result: {:?}", command.content.blocks);
    };
    assert_eq!(name, "codex.shell");
    assert_eq!(tool_use_id, id);
    assert_eq!(input["command"], "echo 1572 > c.txt");
    assert_eq!(input["cwd"], "/opt/moatless/workspace/step0/cproj");
    assert_eq!(*is_error, Some(false));
}

/// A derived line with no call open is shown on its own rather than dropped.
#[test]
fn a_derived_line_with_no_open_call_names_none() {
    let mut calls = LineContext::default();
    let line = CODEX
        .lines()
        .find(|l| l.contains("\"CommandExecution\""))
        .unwrap();
    let mut read = CodexSessionFormat.read_line(line);
    calls.observe(&mut read);
    assert!(read.derived);
    assert_eq!(read.content.derived_from, None);
}

/// Codex names its model on `turn_context`, not on a reply; the tail carries
/// it onto every assistant line after it.
#[test]
fn a_codex_assistant_line_carries_the_turn_context_model() {
    let mut context = LineContext::default();
    let mut lines = read_all(&CodexSessionFormat, CODEX);
    let first_turn = lines.iter().position(|l| l.kind == "turn_context").unwrap();
    for line in &mut lines {
        context.observe(line);
    }
    let assistant: Vec<_> = lines
        .iter()
        .filter(|l| l.content.role == Some(Role::Assistant))
        .collect();

    assert!(!assistant.is_empty());
    assert!(
        lines[..first_turn]
            .iter()
            .all(|l| l.content.role != Some(Role::Assistant))
    );
    for line in assistant {
        assert_eq!(line.content.model.as_deref(), Some("gpt-6-astra"));
    }
}

#[test]
fn only_a_finished_final_response_ends_the_turn() {
    let claude = read_all(&ClaudeSessionFormat, CLAUDE_THINKING);
    let ending: Vec<&RecordedLine> = claude.iter().filter(|l| l.content.ends_turn).collect();
    assert!(!ending.is_empty());
    for line in &ending {
        assert_eq!(line.content.role, Some(Role::Assistant));
        assert!(
            line.content
                .blocks
                .iter()
                .all(|b| matches!(b, CanonicalContentBlock::Text { .. }))
        );
    }
    // The final response's thinking line carries `end_turn` too; an
    // interruption after it has shown no reply.
    let thinking_first = claude.windows(2).find(|pair| {
        pair[0].harness_item_id == pair[1].harness_item_id
            && pair[1].content.ends_turn
            && matches!(
                pair[0].content.blocks.as_slice(),
                [CanonicalContentBlock::Thinking { .. }]
            )
    });
    assert!(
        !thinking_first.expect("a thinking line before the reply")[0]
            .content
            .ends_turn
    );
    assert!(claude.iter().any(|l| {
        !l.content.ends_turn
            && l.content
                .blocks
                .iter()
                .any(|b| matches!(b, CanonicalContentBlock::ToolUse { .. }))
    }));

    let codex = read_all(&CodexSessionFormat, CODEX);
    for line in &codex {
        assert_eq!(
            line.content.ends_turn,
            line.kind == "event_msg/task_complete",
            "{}",
            line.kind
        );
    }
    assert!(codex.iter().any(|l| l.content.ends_turn));
}
