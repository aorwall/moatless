# Rust guidance audit

- **Updated:** 2026-07-25
- **Scope:** `.claude/rules` guidance for Rust under `backend/`, `sandbox/`, and
  `crates/`

## Outcome

The seven path-scoped Rust rule files were not migrated verbatim. They mixed
durable repository decisions with generic Rust advice, rigid templates, and
claims contradicted by current upstream guidance. Their useful content now lives
in the model-invoked
[`rust-dev-guidelines`](../.agents/skills/rust-dev-guidelines/SKILL.md) skill.
The skill keeps universal rules short and loads backend, sandbox, or shared-crate
detail only when the changed path requires it.

Current code is not treated as proof that a convention is good. Verified
violations of retained guidance should be tracked as implementation tickets.
Rejected rules generate no remediation work.

## Primary-source baseline

| Area | Finding used by the skill | Source |
|---|---|---|
| Blocking work | Async tasks must not block the Tokio worker; blocking work belongs behind an appropriate blocking boundary. | [Tokio tasks](https://docs.rs/tokio/latest/tokio/task/) |
| Mutex choice | A standard mutex is often appropriate in async code when its guard does not cross `.await`; Tokio's mutex is for holding a lock across asynchronous work. | [Tokio `Mutex`](https://docs.rs/tokio/latest/tokio/sync/struct.Mutex.html) |
| Axum extractors | Only an extractor that consumes the request body must be last; other extractors run left to right. | [Axum extractor order](https://docs.rs/axum/latest/axum/extract/index.html#the-order-of-extractors) |
| Conversions | `From` is for infallible conversion; fallible conversion uses `TryFrom`. | [`From`](https://doc.rust-lang.org/std/convert/trait.From.html), [`TryFrom`](https://doc.rust-lang.org/std/convert/trait.TryFrom.html) |
| Lint suppression | Lint attributes support native reasons, and `expect` reports an unfulfilled expectation. | [Rust lint levels](https://doc.rust-lang.org/rustc/lints/levels.html), [diagnostic attributes](https://doc.rust-lang.org/reference/attributes/diagnostics.html) |
| SQLx row mapping | Derived `FromRow` maps by column name, so projection policy depends on the row contract and query shape rather than a universal ban on `SELECT *`. | [SQLx `FromRow`](https://docs.rs/sqlx/latest/sqlx/trait.FromRow.html) |

## Migration decisions

### Retained and narrowed

- Keep blocking filesystem, process, and CPU work off Tokio workers; choose mutex
  type from whether the guard crosses `.await`.
- Supervise tasks that own servers, consumers, lifecycle transitions, durable
  synchronization, admission, or required cleanup.
- Make queue acknowledgment, durability, retry, deduplication, capacity, and
  shutdown semantics explicit.
- Preserve error sources through reusable/domain layers; redact and log once at
  the transport boundary.
- Use fallible conversion for unknown or unrepresentable wire values.
- Require native reasons for lint suppressions and prefer exact `#[expect]`.
- Keep visibility narrow.
- Backend-specific: inject handler-facing persistence/domain ports, bind SQL
  values, maintain pagination invariants, and put Axum body extractors last.
- Sandbox-specific: make event acknowledgment match durability, bound the local
  outbox at producer admission, supervise lifecycle work, and terminate protobuf
  DTOs at adapter boundaries.
- Shared-crate-specific: treat serialized event strings and protobuf
  discriminants as wire contracts, validate open enum integers, and feature-gate
  OpenAPI dependencies.

### Rejected

These former rules are not normative and produce no tickets:

- ban all `Box<dyn Error>`, `Rc`, standard mutexes, or synchronous filesystem
  helpers;
- require Tokio locks or `RwLock` for every async state holder;
- require a fixed seven-file backend module layout or logic-free `mod.rs`;
- require every repository to use `async_trait`, `mockall`, `Default` filters, or
  all-`Option` patches;
- ban every `SELECT *` or require `COUNT(*) OVER()` for every page;
- impose a fixed order on non-body Axum extractors;
- require a single error-enum shape or `thiserror` everywhere;
- require every event publish to be fire-and-forget;
- require `Display`, `TryFrom<&str>`, and `as_str` for every enum;
- impose channel capacity `256` repository-wide.

## Verified remediation backlog

The audit checked only rules that survived into the new skill. It did not perform
an unrelated general Rust review.

| Ticket | Priority | Verified problem |
|---|---:|---|
| EVT-1 | P0 | Sandbox event ingress can acknowledge before required durable admission and records deduplication too early. |
| EVT-2 | P1 | The sandbox outbox advertises bounds behind an unbounded producer channel and has an uncapped lifecycle class. |
| DB-1 | P1 | Task pagination can count tag-join duplicates and reports zero beyond the final page. |
| ASYNC-1 | P2 | Sandbox async initialization performs recursive synchronous filesystem work on a runtime worker. |
| WIRE-1 | P2 | Partial protobuf conversions silently collapse unrepresentable or unknown values. |
| SUP-BE | P1 | Backend root tasks can terminate or panic without affecting health or process ownership. |
| SUP-SB | P1 | Sandbox root/durability tasks and some dispatch work are detached from health and shutdown. |
| ING-1 | P1 | Provider HTTP acknowledgment has no durable work-admission seam. |
| ING-2 | P1 | Slack and Telegram acknowledge before required inbound routing is durable. |
| ING-3 | P1 | Required GitHub and Linear webhook work is detached after acknowledgment. |
| PORT-1 | P2 | Handlers construct binding and adapter-connection PostgreSQL repositories directly. |
| PORT-2 | P2 | Remaining handlers perform persistence work through the raw pool. |
| ERR-1 | P2 | Core backend error families erase causal sources into strings. |
| ERR-2 | P2 | Adapter and binding error families erase causal sources into strings. |
| LINT-1 | P3 | Rust sources contain 111 textual `#[allow(...)]` sites and no native suppression reasons. |

Event ingress and local outbox capacity remain separate tickets: the former is a
cross-process durability/acknowledgment contract, while the latter is a local
memory and overload contract.
