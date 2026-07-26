# ING-3 — Migrate GitHub and Linear required webhook work

- **Priority:** P1
- **Status:** Ready for agent
- **Blocked by:** ING-1

## What to build

Route required GitHub message, close, post-merge, and merge-conflict work plus
Linear session provisioning through durable adapter admission before provider
acknowledgment.

## Acceptance criteria

- [ ] Required work crosses the durable handoff before success is returned.
- [ ] Long-running work stays outside the provider response deadline.
- [ ] Provider idempotency keys prevent duplicate effects.
- [ ] Permanent failures reach a terminal, observable state.
- [ ] Best-effort outbound status/activity notifications remain outside scope.

## Verification

- Crash/restart recovery for every admitted work kind.
- Duplicate GitHub and Linear delivery.
- Terminal handling of a permanent failure.
