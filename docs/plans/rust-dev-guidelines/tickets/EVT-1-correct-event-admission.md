# EVT-1 — Correct event admission and acknowledgment

- **Priority:** P0
- **Status:** Ready for agent
- **Blocked by:** None

## What to build

Implement ADR-0006 across sandbox gRPC ingress, event publication, deduplication,
and message persistence. Backend success must mean the complete event payload is
durably admitted. Keep the full `agent.message` payload in a transient durable
outbox until its consumer commits `task_messages`; do not restore it as a
permanent `task_events` copy.

## Acceptance criteria

- [ ] Publication exposes a typed admitted/retryable-failure outcome.
- [ ] Missing or failed required delivery returns a retryable gRPC status unless
      the complete payload was durably admitted.
- [ ] Deduplication marks success only after admission and handles concurrent
      duplicates.
- [ ] Retries reuse a stable non-empty event id and sequence.
- [ ] State-bearing events commit state before fan-out.
- [ ] The outbox relay is supervised, retryable, observable, and deletes only
      successfully delivered records.
- [ ] Full `agent.message` content reaches `task_messages` exactly once.
- [ ] “Transactional outbox” is used only for the durable payload-and-relay path.

## Verification

- Redis disabled and Redis fail-then-recover cases.
- Lost gRPC response after successful admission.
- Concurrent duplicate requests fan out once.
- Full message persistence and state-before-publication ordering tests.
