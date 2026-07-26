# Sandbox event ingress acknowledges durable admission

The backend acknowledges a sandbox-published event only after the complete
dispatch payload has reached the durability required by that event kind.
Deduplication records successful admission, not receipt. A retry of the same
event reuses its stable, non-empty event id and must not create duplicate
downstream effects.

For events whose required state can be committed directly, the state commit
precedes fan-out. For transient dispatch payloads such as full `agent.message`
content, the backend stores the payload in a durable ingress outbox before
returning gRPC success. A supervised relay publishes admitted payloads to the
existing consumer path with retry and removes them only after successful
delivery. The payload remains a transient spool: `task_messages` is still the
canonical message record and `task_events` still stores only the metadata
skeleton described by ADR-0003.

If durable admission fails, or a required delivery path is disabled without a
durable substitute, the backend returns a retryable gRPC error. The sandbox
retains and retries the event. Losing a response after successful admission is
safe because concurrent and repeated requests converge on the same event id.

## Considered options

**Acknowledge Redis publication.** Rejected: Redis unavailability currently lets
the backend acknowledge permanent loss, and a successful transient publish does
not protect the payload from a process failure before canonical persistence.

**Persist full `agent.message` content in `task_events`.** Rejected: this restores
the duplicate permanent message copy removed by ADR-0003. The durable ingress
payload is a retry spool, not the canonical event-history representation.

**Let the sandbox retry without backend durability.** Rejected: a successful gRPC
response is the sandbox's delivery boundary. Retrying only transport failures
cannot recover an event that the backend accepted and then lost.

## Consequences

- A successful publish response means the event can survive backend and Redis
  process failure until its required consumer commits it.
- The ingress outbox requires schema, relay ownership, retry/backoff,
  observability, and cleanup behavior.
- Duplicate and response-loss paths become normal idempotency cases.
- Sandbox-local memory bounds remain a separate concern; EVT-2 defines overload
  behavior before an event reaches this durable backend boundary.
- This ADR refines ADR-0003's transient-ingestion design without changing
  `task_messages` or the `agent.message` skeleton as canonical records.
