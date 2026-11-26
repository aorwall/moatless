---
name: harness-development
description: "Adds coding-agent harnesses to Moatless. Use when integrating another agent CLI or SDK, implementing its runtime or session lifecycle, or wiring a new agent into the catalog. Not for adding a model to an existing provider, upgrading a CLI version, or diagnosing a stuck sandbox."
---

# Harness development

Produce a selectable Agent profile that runs the real CLI in a sandbox and
preserves its conversation through Moatless's Turn and Session lifecycle.

## 1. Establish the native contract

Read the root and sandbox `AGENTS.md`, `CONTEXT.md`, and
`docs/deferred-changes.md`. Use `rust-dev-guidelines` before Rust changes,
`e2e-testing` for the integration suite, and `moat-cli` for deployed checks.

Inspect the upstream package and protocol at the version being integrated.
Verify the package name against the upstream repository; a similar executable
name does not establish its publisher. Record these decisions before coding:

| Question | Evidence to obtain |
|---|---|
| Transport and lifetime | RPC, JSON stream, SDK or terminal; one process per Turn or a persistent process |
| Input consumption | Which event means the agent consumed a prompt, including queued steering |
| Completion | Which event means all tools and follow-up work have settled |
| Session identity | Native session ID, file format, resume and fork behavior |
| Cancellation | Startup cancellation, queue clearing, tool children, and resume after interruption |
| Authentication | Supported API-key, hub, or subscription flows; inherited environment and config precedence |
| Capabilities | Models, effort, modes, skills, extensions, approval dialogs and headless behavior |

Default to a native headless protocol through the existing driver. Introduce
durable workflow machinery only when native sessions plus Moatless recovery
cannot satisfy the requested behavior. Do not advertise unsupported interactive
or extension capabilities.

Define a reproducible success check: a real agent executes a tool, completes a
Turn, and recalls a conversation-only value on a later Turn.

## 2. Implement at the existing runtime seam

Read `crates/sandbox/src/agent/harness/{process,signal,driver,runtime}.rs`.
Implement the private `AgentProcess` port under `harness/<agent>/`; callers
continue using the concrete `HarnessRuntime`. Keep native protocol parsing in
the agent module and shared event ordering in the driver. Wire construction in
`crates/sandbox/src/agent/compose.rs`.

Choose `driver::run` for one execution per process or `run_persistent` for
native execution boundaries within a surviving process. Pi is the RPC example;
Claude and Codex demonstrate other lifetimes. Copy contracts, not their
provider-specific assumptions.

- Publish the real session identity through `SessionReady`. A successful RPC
  response can mean only “queued”; emit `InputAccepted` on consumption so the
  backend does not acknowledge a message the agent never ran.
- Preserve `ExecutionStarted → InputAccepted → Source* → ExecutionFinished`.
  Live steering joins the existing execution without opening another Turn.
  Keep unconsumed inputs available for redelivery or reject them on cancel.
- Use the native settled boundary, not the first end-looking event. Pi's
  `agent_end` precedes `agent_settled`; ending on it can lose queued work.
- Make startup interruptible through `cancel_handle`. Keep the stream reader
  cancellation-safe when the driver selects between output, steering and stop;
  retain partially read bytes and bound the frame size.
- Clear native queued inputs before abort where supported. Stop tool children
  as well as the CLI, wait with a deadline, and flush trajectory data before
  publishing termination. On Unix, use a process group and direct safe signal
  calls such as `nix::sys::signal::killpg`, not a shell spawned to run `kill`.
- The driver owns `Outcome::Cancelled`. `ProcessTermination::Completed` during
  requested cancellation is not a successful user Turn; do not invent a second
  cancellation mapping in the adapter.
- Classify failures with `HarnessError` stage, class and session impact. An
  authentication failure is not a lost session. Report new-run usage, not
  cumulative session totals that the backend would count again.

## 3. Preserve native history

Implement `SessionFormat` from `harness/trajectory.rs`, and use
`TurnTrajectory` in `crates/sandbox/src/trajectory_tail.rs` to restore, record
and flush. Live RPC events drive activity and runtime output; persisted
conversation history comes through the native transcript tail. Writing it
through both paths duplicates messages and usage.

- Handle fork, then harness switch, then fresh start, then ordinary resume,
  following `Input`'s precedence. A fork or switch also carries `start_fresh`;
  checking it first discards the history being transferred.
- Preserve native tool IDs, parent links, signatures and compaction entries.
  Retarget only identity fields required for a fork. For cross-harness history,
  render valid native entries through the existing transcode path.
- Missing requested history must not silently become a blank conversation.
  Verify native resume on disk and restoration from the backend separately.
- Inspect `crates/tasks/src/trajectory/lineage.rs` when defining fork cuts.
  A following user line can reach the tail before its Turn-start event and be
  stamped with the prior Turn number; the native end boundary must keep it out.
  Preserve required results for tool calls already inside the cut.
- Inspect `crates/server/src/sandbox/grpc_server/messages.rs` for resume policy
  after stopped or errored Turns. Retaining a local file is insufficient if
  the backend clears the session ID before a replacement sandbox starts.
  Preserve an interrupted session only when the agent can safely resume it.

## 4. Wire selection, credentials and distribution

Trace each surface below. Exhaustive enum matches catch some omissions, but
catalog, SQL constraints and UI model lists can remain incomplete while compiling.

| Surface | Starting points |
|---|---|
| Harness family and profile strings | `crates/types/src/agent.rs`, including parsing, serialization and `ALL` |
| Models, defaults, credential requirements | `crates/core/src/agent_catalog.rs` |
| Availability and switching | `crates/tasks/src/agent/service.rs`, `task/switch.rs`, `message/enricher.rs` |
| Credential routing | `crates/server/src/model_gateway/`, sandbox credential environment |
| T3 provider projection | `crates/t3code/src/{providers,config,owners}.rs`, `crates/server/src/t3code/backend/commands.rs` |
| Persisted harness constraints | `crates/core/migrations/`, especially trajectory and conversation tables |
| CLI selection | `crates/cli/src/commands/`, `moat tasks create --help` |
| Packaged executable | `docker/{cli-versions.env,Dockerfile.sandbox-base,build-sandbox.sh}`, `.github/workflows/ci.yml` |
| Upgrade verification | `.moatless/loops/agent-cli-upgrade/LOOP.md` |

Keep credentials in `RUNTIME_DIR`, outside workspace snapshots. Layer the
coordinator's per-Turn environment and system prompt into the process, including
its Member's `TASK_ID`. Isolate the native agent config and explicitly load
managed skill roots so host logins or arbitrary extensions cannot change the
tested authentication or lifecycle behavior.

For an API-key/hub-only harness, exclude inherited OAuth credentials and native
login files. Gateway selection must also enforce that policy before refreshing
subscription credentials; stripping environment variables alone is insufficient.
Pi's `X-Moatless-Auth-Mode` is a restrictive policy, not authorization, and is
removed before forwarding upstream. Use the configured gateway and its existing
credentials; do not copy a developer's login file into the checkout or image.

Resolve T3 client support in the separate T3 Code client repository if the
new provider needs client changes. Backend catalog exposure alone does not
prove that the client can select it.

Pick the next unused migration number from the target branch. Recheck after
rebasing: two independently added migrations can have the same numeric prefix
without producing a textual merge conflict.

## 5. Verify the real lifecycle

Use shared harness contract tests for ordering, early output, queued input,
EOF, startup cancellation and exactly one terminal outcome. Add native-format
tests for restore/fork/transcode and a process-group test that detects a tool
child surviving cleanup. Run the root checks through `rust-dev-guidelines`;
do not treat a filtered harness pass as a full sandbox-suite pass.

Use `e2e-testing` to add a capability-gated spec in `tests/api/specs/` and run
it through `scripts/local-e2e/run.sh` against binaries built from the checkout.
`agent-pi-flow.spec.ts` is a worked example. Cover each supported credential
route and provider with these observations:

| Scenario | Evidence |
|---|---|
| Real tool and follow-up | Observable file contents change on successive Turns |
| Fork at an earlier Turn | Recalls an earlier conversation-only value but excludes a later value |
| Sandbox replacement | Recalls a value after stopping the sandbox and removing the local session copy in the isolated test fixture |
| Live steering | Consumes the queued instruction in the same Turn |
| Cancellation | A long-running tool cannot finish its delayed side effect; the next Turn retains history |
| Harness switch and switch back | Preserves conversation-only values across both transitions |

Check capability skips explicitly. A green suite that skipped the new agent is
not evidence of integration. Never delete session files from a person's task
to simulate recovery; that belongs in the disposable E2E fixture.

The installed CLI can lag the backend and reject a released profile. Compare
its help with the checkout CLI, and follow `moat-cli` to build the latter when
needed. Ensure the E2E runner selects checkout binaries on `PATH` too.

After release, use `moat-cli` to check the catalog, create a dedicated smoke
task, execute a harmless tool, and verify a second-turn memory recall. Inspect
messages and agent state: a Task remaining `open` says nothing about completion.
A ready pod may still be running workspace setup. If startup stalls, use
`sandbox-troubleshooting` to distinguish scheduling, image pull, setup, protocol
startup and provider failure before changing runtime code.

## Completion

Report the implementation checks, real-agent scenarios and skips separately.
Verify the Linux CI result and the sandbox image build; passing on the host
does not establish that the image contains the CLI. Record deliberately
deferred capabilities and their re-entry conditions in `docs/deferred-changes.md`.

For evaluating this skill's routing and execution, read
[evaluation scenarios](references/evaluation.md).
