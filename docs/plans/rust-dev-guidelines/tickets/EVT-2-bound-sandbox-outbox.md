# EVT-2 — Enforce real sandbox outbox bounds

- **Priority:** P1
- **Status:** Ready for agent
- **Blocked by:** EVT-1 for final retry semantics; local work may begin
  independently.

## What to build

Replace the sandbox outbox's unbounded producer admission with enforceable
count/byte bounds. Canonical and lifecycle events use bounded,
cancellation-aware backpressure or a finite failure policy. Only explicitly
lossy telemetry may drop, and every drop must be measured.

## Acceptance criteria

- [ ] Capacity is enforced when the producer submits, including while the
      drainer is stalled.
- [ ] Lifecycle saturation has a finite, documented policy; no priority class is
      uncapped.
- [ ] Non-lifecycle eviction and marker ordering remain deterministic.
- [ ] Drainer termination is supervised and shutdown is cancellation-aware.
- [ ] Graceful flush remains bounded and best-effort.
- [ ] Metrics distinguish dropped, backpressured or failed, retried, and
      undrained events.

## Verification

- Stalled backend with producer bursts at count and byte limits.
- Lifecycle-only saturation and priority eviction.
- Marker ordering, cancellation during retry, and bounded flush timeout.
