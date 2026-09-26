# Transcode fixtures

One conversation recorded in each harness, and each rendered into the other
harness's format. `trajectory_e2e.rs` resumes the rendered files with the real
CLIs (`claude_to_codex_resume_knows_history` and its siblings), and phase 2's
`SessionFormat::render` is tested against them. The plan is
`.plans/harness-switching.md`.

| File | What it is |
|---|---|
| `claude.jsonl` | Claude Code 2.1.281, `claude-haiku-4-5`: Turn 1 `Write`s `release.txt` (PERIWINKLE), Turn 2 `Read`s it, `/compact`, Turn 3 names SAFFRON. Lines 0–38 are Turns 1–2. |
| `codex.jsonl` | Codex 0.156.1, `gpt-5.5`: Turn 1 `exec_command` writes `release.txt`, Turn 2 `apply_patch` appends NOTES, an auto-compaction, Turn 3 names SAFFRON. Lines 0–37 are Turns 1–2. |
| `codex-from-claude.jsonl`, `claude-from-codex.jsonl` | Turns 1–2 rendered into the other format. |
| `*-compacted.jsonl` | All three Turns rendered, from the last compaction (S5). |
| `spike_transcode.py` | Writes the four rendered files from the recordings. Phase 2 deletes it. |

The recordings ran in `/workspace`; the rendered files carry `{{CWD}}` and
`{{SESSION_ID}}`, which a test replaces before writing the file.
The recordings' Claude `prompt_snapshot` attachments and Codex
`<skills_instructions>` are trimmed stubs: the CLI's own prompt and the
recording sandbox's skills, which no transcode reads.

## ToolMap

| Claude Code call | Becomes in Codex |
|---|---|
| `Bash {command}` | `function_call exec_command {cmd}` |
| `Write {file_path, content}` | `custom_tool_call apply_patch`, `*** Add File:` |
| `Edit {file_path, old_string, new_string}` | `custom_tool_call apply_patch`, `*** Update File:` with one `@@` hunk |

| Codex call | Becomes in Claude Code |
|---|---|
| `exec_command {cmd}`, `shell`, `local_shell` (a `["bash","-lc",cmd]` argv becomes `cmd`) | `Bash {command}` |

Any other call, in either direction, becomes one assistant text item naming
the tool, its input and its result, and its result line is dropped. A call's id
is kept as the other harness's id (`toolu_…` as a Codex `call_id`, `call_…` as a
Claude `tool_use.id`); both CLIs accept the other's shape.

## What each CLI requires of a line it did not write

- **Claude Code** resumes a file of `user`/`assistant` lines alone. Each needs
  `type`, `uuid`, `message` and a `parentUuid` naming the line before it:
  without the chain it sees only the last line. `cwd` and `sessionId` are not
  read on resume; the renderer writes them so the file reads like the CLI's own.
- **Codex** refuses a rollout whose line 0 is not `session_meta`
  (`does not start with session metadata`). No `turn_context` or `event_msg`
  line is needed. It tolerates a call without its output and an output without
  its call.
- Neither needs the other harness's reasoning; `thinking` and `reasoning` are
  dropped (decision 1).

## Injected lines

What the harness wrote into the user role itself, which `read_line` marks
`injected` and the transcode drops:

- Claude: `isMeta` lines, and user text starting `<command-name>`,
  `<command-message>`, `<local-command-stdout>`, `<local-command-caveat>` or
  `<system-reminder>`; `attachment` lines have no content.
- Codex: `developer` messages, and user text starting `<environment_context>`,
  `<user_instructions>`, `<skills_instructions>`, `# AGENTS.md` or
  `<turn_aborted>`.

## Compaction lines (S5)

- Claude: a `system` line with `subtype: compact_boundary`, then a `user` line
  with `isCompactSummary: true` whose text is the summary. The boundary's
  `compactMetadata.preservedSegment` keeps the last assistant reply verbatim
  before the boundary; the summary covers it, so the transcode starts at the
  summary.
- Codex: a `compacted` line; `payload.message` is the summary and
  `payload.replacement_history` the items the CLI kept.
