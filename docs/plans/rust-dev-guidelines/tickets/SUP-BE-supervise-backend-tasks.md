# SUP-BE — Supervise backend root tasks

- **Priority:** P1
- **Status:** Ready for agent
- **Blocked by:** None

## What to build

Give the backend root explicit ownership and criticality policies for its
servers and long-lived workers. The audit includes the metrics server, SSE Redis
consumer, backend gRPC server, event consumer, sandbox reconciler, runtime
watchers, task reaper, Codex refresher, cron scheduler, and Telegram poller.

## Acceptance criteria

- [ ] `main` or one root supervisor owns critical join handles.
- [ ] Early return and panic become structured failures that terminate the
      process or change readiness according to declared criticality.
- [ ] Optional worker termination policy is explicit rather than inferred from a
      discarded handle.
- [ ] Root cancellation reaches each owned task.
- [ ] Shutdown joins tasks within a bounded deadline and reports stragglers.

## Verification

- Synthetic critical panic and early server return.
- Optional worker exit policy.
- Cancellation, bounded join, and readiness/fatal behavior.
