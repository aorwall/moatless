# Sandbox Rust

Apply this reference to Rust changes under `crates/sandbox/`.

## Lifecycle and Supervision

- Long-lived services, event delivery, agent/lifecycle owners, and durability
  loops use the lifecycle cancellation token and surface abnormal termination
  to process health or a supervisor.
- Agent completion and failure paths both leave state consistent.
- Follow the [file responsibilities](file-roles.md) loaded from `SKILL.md`.
  Keep behavior needed by integration tests callable through the library.
- Choose synchronization from the access pattern; do not require `RwLock` or an
  async mutex by default.

## Event Delivery

- Event retries reuse a stable non-empty `event_id` and sequence.
- Backend acknowledgment means the event reached the durability required by its
  kind. A missing or failed required downstream returns a retryable error.
- Deduplication records successful admission, not mere receipt.
- State-bearing events commit state before fan-out.
- Conversation content reaches the backend only as trajectory lines; an append
  is acknowledged once the lines and their projection commit together.
- The sandbox outbox is ordered, in-memory, and non-durable across process
  death. Producer-side count/byte limits and priority behavior must remain
  enforceable while its drainer is stalled. Graceful flush is bounded and
  best-effort.

## Boundaries

- Protobuf DTOs terminate at adapter boundaries.
- Use `TryFrom` for unknown or unrepresentable protobuf values and propagate
  recursive conversion failures with context.
- Put conversion code beside the more specific domain or adapter type rather
  than automatically in the protobuf crate.
- Keep synchronous filesystem staging directly unit-testable, but invoke it
  from async initialization through bounded blocking work.
