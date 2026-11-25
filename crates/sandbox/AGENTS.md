# moatless-sandbox

Rust sandbox runtime for Moatless sandbox pods.

Each module's `//!` doc states what it owns.

A long-lived task registers with the root supervisor and uses lifecycle
cancellation. The agent command loop is the one exception: it reports
liveness through channel closure and exits when its handles drop, so
registering it would make every shutdown report it as a straggler.

## Spawning a child process

Give every child process this sandbox spawns for a Member that Member's
`TASK_ID`, and overwrite whatever the caller supplied. The pod's own `TASK_ID`
is written once at creation and names the Member that started it, so a shared
sandbox would otherwise run the second Member's `moat gh pr create` against the
first Member's Task. `TASK_ID` is in `RESERVED_SECRET_KEYS`, so this is an
`insert` and not the `or_insert` that `CLAUDE_ENV_DEFAULTS` uses.

Three sites spawn for a Member and stamp. `init::workspace::run_setup_command`
is the fourth spawn and stamps nothing: init runs only for the pod's initial
Member, whose pod `TASK_ID` already names it.
`agent::coordinator::service::build_input` stamps the `Input::env` that every
harness layers over the inherited environment.
`CommandRegistry::run` stamps the `bash -lc` child, and takes the Task as a
constructor argument so no registry can be built without one.
`terminal::runtime::spawn_terminal` applies whatever env the request carried,
and `moatless_t3code::terminal` stamps it there, after the Task the caller is
authorized for is resolved.

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
the user's `git status`.

## Storing credentials

Credential material on disk belongs in `RUNTIME_DIR`, never `WORKSPACE_DIR` or
`DATA_DIR`: the sandbox and the agent subprocess share a container and a uid,
so keeping it out of those two directories bounds exposure rather than making
it unreadable to the agent.

Keep credentials out of `$CLAUDE_CONFIG_DIR/settings.json` too: its `env` block
outranks the process environment, so a credential written there is neither in
`RUNTIME_DIR` nor readable only by the agent.

## Surviving a pod replacement

The registry, the watch, and any in-flight run all die with the pod. A
replacement recovers them through three owners: `CommandRegistry`'s journal and
backend mirror recover unreported `moat cmd` commands (`take_lost_commands`), a
**wake note** (`agent::wake_note`) recovers text an in-pod caller left for the
agent, and `dispatcher::queue_and_dispatch` is the only path that delivers
either as a recoverable turn — never `agent_handle.send`, which a pod that dies
mid-turn cannot resume. Tests cover the ordering and dedup rules that make each
of these safe.
