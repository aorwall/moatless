# ING-1 — Add durable adapter work admission

- **Priority:** P1
- **Status:** Ready for agent
- **Blocked by:** SUP-BE must own the resulting worker.

## What to build

Create one durable, idempotent admission seam for provider webhook work that
cannot safely finish on the HTTP response path. Provider success must mean the
required work is complete or durably handed off, not merely spawned.

## Acceptance criteria

- [ ] Persist an idempotent work record or equivalent durable handoff keyed by
      the provider event identifier.
- [ ] Define retry/backoff, terminal failure, observability, and poison-item
      isolation.
- [ ] A supervised worker owns admitted work and recovers it after restart.
- [ ] Provider deadlines remain respected because long processing stays off the
      response path.
- [ ] Best-effort outbound activity notifications and API-key usage timestamps
      remain outside this durable path.

## Verification

- Crash after HTTP acknowledgment.
- Process-restart recovery.
- Duplicate webhook and poison-item isolation.
