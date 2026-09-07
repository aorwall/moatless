# moatless-sandbox

Rust sandbox runtime for Moatless Vibe sandbox pods.

Each module's `//!` doc states what it owns. Two process rules bind all of them:

- `SandboxState` is shared through axum `State`. Keep its fields cheap to clone,
  with `Arc` or internal sharing.
- A long-lived task registers with the root supervisor and uses lifecycle
  cancellation. The agent command loop instead reports liveness through channel
  closure and exits when its handles drop; registering it would make every
  shutdown report it as a straggler.

## How the sandbox and the backend talk

`sandbox.proto` defines both directions: the backend's calls into
`SandboxService` and the sandbox's calls into `BackendSandboxServiceClient`.
HTTP serves health probes only. Keep runtime control on gRPC.

## Serving a preview

`preview_proxy.rs` is the pod's second inbound listener, on
`SANDBOX_PREVIEW_PROXY_PORT`. Every port a Script publishes reaches it, and it
forwards over loopback so the server sees a loopback `Host`. It is not the
control router, and preview traffic is untrusted: keep it a separate listener
that reaches only a non-reserved port.

Non-reserved means `moatless_types::sandbox::is_reserved_sandbox_port`, the one
rule that this proxy and `sandbox-k8s`'s `pod_builder::admitted_server_ports`
both read. Do not copy it into a second list.

"Loopback" is the whole pod's network namespace, which the server sidecars and
`dind` share, so a new sidecar is reachable from the proxy the day it is added.
Keep one out of reach by reserving its port.

## Configuring the pod

`Config` in `config.rs` is the authority for every environment variable, its
default and its meaning. Each field carries a `///` doc, and `from_env` holds
the defaults. Read it there rather than from a list here.

Two values are not configurable. `RUNTIME_DIR` is one; see Storing credentials
below. `SANDBOX_PREVIEW_PROXY_PORT` is the other: the Service's `targetPort` and
the proxy's listener are two ends of one wire, set by different processes, so a
per-sandbox value would let them drift with nothing to detect it.

## Publishing events

Publish every event through `EventOutbox`. Do not bypass its bounded retry and
drop accounting.

At capacity the outbox evicts the oldest evictable event and counts
`dropped_overflow`. When only lifecycle events (`taskTurn.*` and terminal
`agent.*`) remain, it refuses the incoming event and counts
`rejected_saturated` instead. It always keeps one event, even when that event
alone exceeds the byte cap.

Lifecycle events are preferred but share the `MAX_QUEUE_EVENTS` and
`MAX_QUEUE_BYTES` budget with every other event, so the outbox refuses the
newest and keeps the oldest — the drainer is FIFO, and an older event is the
prefix that keeps the later ones interpretable.

Drop an event the backend *permanently* refuses; do not retry it. That event is
at-most-once, because one in-order drainer would otherwise stall on it and take
the task's whole remaining stream with it.

Each loss cause has its own counter, and each calls for a different response:

| Counter | What it reports |
|---|---|
| `dropped_overflow` | A load condition |
| `rejected_saturated` | A load condition |
| `dropped_terminal` | A wire-contract violation |
| `dropped_unrepresentable` | A bug in this process |

## Running a turn

- `dispatch_pending_messages()` is the only path that fetches, submits and
  acknowledges a pending message. Call it from init, from gRPC notify, and after
  the agent completes.
- `CoordinatorService` mints a `run_id` (UUID) when a run starts while the agent
  is idle, and holds it for the run's lifetime. Every event published inside the
  run carries it, and so does the message acknowledgment. The backend maps a
  `run_id` to a turn number, so a turn is exactly one agent run whatever the
  delivery order.
- Interactive prompts block on `tokio::sync::oneshot`, for `AskUserQuestion` and
  `ExitPlanMode`.
- A restart wake resumes the prior session. `build_input` resolves the resume
  target in order: the pod's last session id, then the backend-provided id, then
  `--continue` for the Claude family. Init restores transcripts to disk before
  Ready. The plain `send(text, None, …, None, false, None)` that
  `report_lost_commands` and `JobInjector` both use then picks the session back
  up with its context.

## Writing files inside the pod

`init/` writes nothing into a checkout under `WORKSPACE_DIR`. A file it creates
there is untracked, and it lands in the user's `git status` and PR diff. Stage
anything an agent must discover in a home or config directory instead:
`$CLAUDE_CONFIG_DIR`, `$CODEX_HOME`, or `/etc/codex/skills`. A repo that wants
`CLAUDE.md` beside its `AGENTS.md`, or `.claude/skills` beside
`.agents/skills`, commits that symlink itself. The clone, the configured setup
commands, and `workspace_sync`'s restore are the only writers inside a
checkout.

A message's attached uploads are written under `UPLOAD_DIR`, at
`$WORKSPACE_DIR/uploads` — a sibling of the repo checkouts and never a path
inside one, so it survives a restart through `workspace_sync` and stays out of
the user's `git status`. The backend owns the value: it renders the same path
into the prompt's "Saved to:" line and passes it as `UPLOAD_DIR`. Do not give
the sandbox an independent default.

The "Saved to:" line is a claim the backend makes before the sandbox holds the
bytes, so a failed fetch must be contradicted rather than logged.
`files::uploads::fetch_uploads_for_messages` returns the `file_id`s that did not
land, and `agent::dispatcher` appends `failed_upload_note`'s text to the turn's
prompt. A fetch failure does not block the turn, but it must never leave the
agent holding a path that was never written.

`BackendClient::download_upload` reassembles a chunk stream
(`DownloadUploadChunked`), so the sandbox's memory bounds an upload's size and
no per-message limit on the wire does. Keep it that way instead of raising a
cap.

## Storing credentials

Credential material on disk belongs in `RUNTIME_DIR` (`/run/moatless`,
`Config::runtime_dir`): the credential broker socket and the persisted user auth
token. It is the only directory the workspace snapshot does not archive and the
server sidecars do not mount. It is not configurable —
`docker/moatless-git-cred.sh` hardcodes the broker socket beneath it.

Use neither `WORKSPACE_DIR` nor `DATA_DIR` for credential material:
`workspace_sync` archives and restores the first, every sidecar mounts the
second, and `credentials::fetch::purge_exposed_user_tokens` sweeps both on every
successful fetch. The sandbox and the agent subprocess share a container and a
uid, so this bounds exposure and does not make a file unreadable to the
agent.

No single store reliably holds the freshest backend auth token, so init tries
them all. `Config::initial_auth_token_candidates` lists them; when the initial
`GetCredentials` is rejected as `Unauthenticated`, init presents the next, and
the failure is permanent (exit 78) only once every candidate is refused. Do not
collapse this to a fixed precedence: whichever store loses becomes a crash loop
on that runtime.

Claude CLI env defaults live in `agent::options::CLAUDE_ENV_DEFAULTS`. They are
applied with `or_insert`, under whatever the credential channel resolved for
the task. A global-scoped or user-scoped `env`-kind secret of the same key
therefore wins. Keep them out of `$CLAUDE_CONFIG_DIR/settings.json`: that
file's `env` block outranks the process environment, so nothing can override a
default written there. `init::claude_code_settings` therefore writes no `env`
block. The same rule bars credentials from that file, which is neither
`RUNTIME_DIR` nor readable only by the agent.

## Frontloading repo instructions and skills

`agent::options::build_repo_instructions_section` inlines each checked-out
repo's root instruction file (`CLAUDE.md` or `AGENTS.md`) into the agent's
system prompt. Both agents' own memory loading excludes the same files: Claude
through `claudeMdExcludes` (`init::claude_code_settings`), Codex through
`project_doc_max_bytes = 0` (`init::codex_config`). The section and the
exclusion both key off `Config::repo_dirs()`. Keep them in sync.

Inlining is the default; a path the agent is merely pointed at does not
reliably get read. `SANDBOX_FRONTLOAD_REPO_INSTRUCTIONS=false`, or
`frontloadInstructions` in `REPO_CONFIG`, reverts to listing paths
(`AgentConfig::frontload_instructions_enabled`).

Read the file through `agent::options::read_capped`. The cap must bound the I/O
itself: the path is repo-controlled and resolves through symlinks, so reading
first and trimming after lets a checkout choose how much memory prompt assembly
spends.

A delivery's `skills` are names, never bodies.
`agent::skills::frontload_skills` resolves each name against
`init::home_skills::REPO_SKILL_ROOTS` under every `Config::repo_dirs()` entry,
then against the harness's own home skills directory
(`$CLAUDE_CONFIG_DIR/skills` or `$CODEX_HOME/skills`). It prepends
`Activated <root>/<name>:` and the `SKILL.md`. Both modules key off that one
const, so keep them in sync. A name that resolves nowhere is warned and skipped,
and the turn still runs.

A delivery carrying only skills and no message text is enough to open a turn.
`dispatcher` gates on `text.is_empty() && skills.is_empty()`, frontloads, then
re-gates on the frontloaded text, so a delivery whose every skill failed to
resolve still opens nothing.

A `PluginDelivery`'s `frontload_skill_names` are the skills that lead every
turn opening a fresh session, whatever that turn asked for.
`agent::skills::with_always_frontloaded` merges them ahead of the delivery's own
names. `dispatcher` merges only when there is no `resume_session_id` — a resumed
session already holds the bodies — and only after the emptiness gate, so a
standing skill cannot open a turn on an empty delivery.

The names arrive resolved: the backend picks them per person and Workspace at
provisioning time, so a flag set afterwards reaches the pod on its next
start.

## Tracking `moat cmd` commands

The `moat-cli` skill states the contract this code owes the agent: an
unconditional wake on any terminal state, outcomes read back explicitly, and a
single announcement after a pod replacement. `commands.rs` owns the registry.
These invariants are what keep that contract true.

`busy_processes` counts only the commands registered through `moat cmd` that are
still inside their deadline. Nothing walks `/proc`, so a process the agent forked
by other means must not defer the idle reaper. Keep the registry the sole writer,
and do not reintroduce process-tree discovery.

The background-completion watch (`agent::background_watch_job`) arms off that
same registry, and `agent::options::build_managed_command_note` promises the wake
unconditionally. The watch must fire in three cases a simpler filter misses:

1. On *any* terminal state: exited, timed out, or killed.
2. On the *first* command to reach one, rather than the last.
3. For a command that reached one *before* the watch armed. A command with a bad
   path dies in under a second, and a watch armed over what is still running
   arms nothing.

The watch list (`unreported()`) and every poll of it (`unreported_among()`) key
on `Entry::reported`, never on process state: a state filter drops the command
that died before arming and picks up one the agent killed, because `kill` sets
`reported` while the process is still `Running`. Re-read `reported` on each
poll. A `Send` does not cancel an armed watch, so a later turn can kill or read
back a command an older watch still polls.

`GetAgentStatus` must keep calling `list` and never `read_for_agent`. The backend
polls status on its own schedule, so a poll that marked a command seen would
swallow the wake.

`grpc_server::run_command` falls back to `workspace_dir` when a request carries
no cwd. Never let it inherit the sandbox process's cwd, `/opt/moatless`, which is
neither the workspace nor a checkout: a relative path would resolve somewhere the
caller never chose.

### Surviving a pod replacement

The registry is in memory, so a replacement destroys every command, its log under
`/tmp`, and the armed watch. `CommandRegistry::with_journal` mirrors the
unreported set to `$WORKSPACE_DIR/.moat-commands.json`, the S3 workspace snapshot
being the only sandbox state that survives one. The journal holds metadata only;
logs stay in `/tmp` (`default_log_dir`), where an unbounded build log cannot fill
the volume whose size limit causes these evictions.

`agent::restart::open_for_work` reads the journal through `take_journal`
**before** anything else, or a recovered run or a new command overwrites it.
`report_lost_commands` turns the journal into the wake the watch could not
deliver. `take_journal` clears the file whether or not it parsed, so the loss is
announced exactly once. Key it on `Entry::reported`, like the watch.

Install the journal by writing a sibling `.json.tmp` and renaming over it, so a
kill or a snapshot racing the write cannot leave a truncated file.

Consuming a journal also suppresses `adopt_current`. The delete is local while
the S3 archive still holds the file, so adopting this tree as the upload baseline
lets a turn that edits nothing skip the upload — and the next replacement then
restores the stale archive and announces the same loss again.
