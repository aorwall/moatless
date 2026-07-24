# One task-creation entry point: `TaskCreationService`

Task creation — generate a unique short id, run GitHub-token preflight, insert
the task row, assign tags, record relations, provision the sandbox, send the
first message, publish a task event — is owned by a single concrete service,
`TaskCreationService` (`backend/src/task/creation.rs`). Callers build a
normalized `NewTaskRequest` describing *what* task to create; the service owns
*how*, in a fixed order:

```
short-id (single retry loop) → preflight → insert_task → tags → relations
  → provision (sandbox.start, best-effort) → optional first message → task event
```

The service is a plain struct (no trait — repo rule: no new trait with a single
implementation), stored as `Arc<TaskCreationService>` on `AppState`, following
the `SkillService` / `SecretService` precedent. It is constructed from the same
repository trait objects and the sandbox lifecycle handle the paths already use,
wired once in `state.rs`. It takes **no `&AppState` parameter anywhere** — so it
is constructible in unit tests from mocks alone, with no database.

Before this, the sequence was copy-pasted across four call paths (HTTP handler,
triggers, GitHub PR adapter, Linear adapter) and had drifted: four byte-identical
short-id generators, four fire-and-forget `provision_task` copies, preflight only
on the HTTP path, and provision-failure surfaced as a `warn!` log on every path.
This ticket lands the service and migrates the first caller — HTTP `POST /tasks`;
tickets 02–04 migrate the remaining three.

## Two seams keep `AppState` out of the service

Two dependencies the sequence needs are themselves `&AppState`-coupled, so each
is inverted behind a one-method port injected into the constructor and
implemented once in production wiring:

- **`PreflightPort`** — `validate(task, repo) -> Result<(), PreflightError>`.
  `RealPreflight` holds exactly the Arcs `preflight_validate_github_token`
  consumes (`secrets`, `provider_tokens`, `github_app_service`,
  `provider_token_auth`, `github_apps`, `users`, `credentials_svc`) plus the
  existing `GithubValidator` seam, gathered into `PreflightCredentials` and built
  in `state.rs` **before** `AppState` assembly. Preflight is therefore wired
  once; tickets 02–04 get it for free with zero call-site wiring — the whole
  point of the consolidation. A per-caller preflight *closure* was rejected for
  the opposite reason: it would re-scatter the exact thing being consolidated.

- **`SandboxStarter`** — `start(task_id) -> Result<(), String>`, implemented by
  the sandbox `Lifecycle` handle. `Arc<Lifecycle>` coerces to
  `Arc<dyn SandboxStarter>` at wiring time; unit tests inject a mock.

Two genuinely caller-specific concerns stay closures passed to `create`, not
service state, because only some callers use them:

- **first message** — only the HTTP path sends one inline; adapters pass `None`
  and deliver via their binding. A `send_first_message(task_id, FirstMessage)`
  closure keeps `send_message_flow`'s `&AppState` dependency in the caller.
- **success-event payload** — the enriched response body is HTTP-shaped; a
  `build_event_payload(task) -> Value` closure builds it after insert, when the
  row (and its generated ids) exists.

## Preflight and provision-error-event consistency

Two behaviour changes are deliberate, not side effects — they are the reason the
sequence is consolidated:

1. **Preflight runs on every path.** On failure the service inserts the row as
   `Error` and publishes a `SandboxStatus::Error` sandbox event (the pattern
   `start_task` already uses), then returns `TaskCreationError::Preflight`. The
   HTTP handler maps that to its existing `403`. Trigger/adapter paths (02–04)
   inherit the same errored-task treatment instead of failing later inside the
   sandbox with a worse failure mode.

2. **Provision failure publishes a sandbox error event on every path.** A failed
   `sandbox.start` was previously log-only, so the task sat silently dead.
   Creation still succeeds (fire-and-forget preserved); the task is just visibly
   errored. The provision result is returned to the caller as
   `ProvisionOutcome::{Started, Failed}` inside `TaskCreationOutcome`, so ticket
   02's `schedule_runner` can keep recording its
   `record_job("error", "provision_failed", …)` metric.

## What stays per-caller / in the handler

- **Per-caller, passed in `NewTaskRequest`:** event subtype (`Created` vs
  `Open`), visibility, tags, first-message strategy. Not normalized — normalizing
  them would be behaviour change beyond the two named ones.
- **Stays in the HTTP handler:** workspace→repo resolution, scope resolution,
  codex credential fast-fail, and response shaping. These are HTTP/pre-normalization
  concerns, not task creation.

## Consequences

- Deleted from `task/handlers.rs`: `insert_and_provision_task` and the local
  `generate_unique_short_id`. Tickets 02–04 delete the other three short-id and
  `provision_task` copies as they migrate.
- `preflight_validate_github_token` now takes `&PreflightCredentials` instead of
  `&AppState`; `start_task` passes `PreflightCredentials::from_state(&state)`.
- The file-scoped `RecordingPublisher` from `sandbox/grpc_server.rs` moves to a
  shared `#[cfg(test)] test_support` module so both the grpc ordering tests and
  the new creation tests use one recording `EventPublisher`.
- The service is unit-tested with no database: `MockTaskRepository` and mocks for
  the other repos, `MockSandboxStarter`, `MockPreflightPort`, and the recording
  publisher cover short-id retry, preflight failure, provision failure, and the
  happy path. The HTTP contract is pinned by the unchanged
  `tests/task_handler_test.rs`.
