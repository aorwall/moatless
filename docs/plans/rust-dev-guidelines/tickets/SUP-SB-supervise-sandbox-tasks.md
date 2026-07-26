# SUP-SB — Supervise sandbox services and dispatch tasks

- **Priority:** P1
- **Status:** Ready for agent
- **Blocked by:** Coordinate outbox ownership with EVT-2.

## What to build

Surface abnormal termination from the sandbox gRPC server, initialization,
credential/transcript/workspace durability loops, agent execution/message
dispatch, and the Codex auth watcher. Reuse the existing dispatcher supervision
seam where applicable.

## Acceptance criteria

- [ ] Root critical tasks feed process liveness or an explicit fail-fast policy.
- [ ] Initialization `JoinError` fails startup instead of disappearing in a
      detached task.
- [ ] Dispatcher RPC tasks use the supervised path while preserving pending
      message durability.
- [ ] Periodic durability-loop failure is observable and either restartable or
      health-affecting by explicit policy.
- [ ] Cancellation and shutdown joins are bounded.

## Verification

- Dispatcher panic is surfaced.
- Early gRPC termination affects health.
- Initialization join failure and periodic-task cancellation cases.
