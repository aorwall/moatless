# 02 — Expand backend Message assembly for per-block source events

**What to build:** Let the backend ingest one canonical source event per native content block and still present one correctly assembled Message through both persisted reads and live upserts. Keep current coalesced Sandbox events compatible during migration so this ticket can land independently.

**Blocked by:** None — can start immediately

**Status:** ready-for-agent

- [ ] Multiple assistant source events with one native Message identity assemble into one UI-facing Message.
- [ ] Assembled content blocks retain their native order.
- [ ] Usage repeated on multiple blocks is admitted exactly once for the assembled Message and downstream cost totals.
- [ ] Duplicate source-event delivery is idempotent and does not duplicate blocks or usage.
- [ ] Interleaved tool results resolve and upsert the affected parent Message.
- [ ] Paginated reads and live derived upserts cross the same assembly seam and produce equivalent Message shapes.
- [ ] Subagent rows remain excluded from top-level Message assembly and retain lazy transcript behavior.
- [ ] Existing coalesced assistant events remain readable during the migration period.
- [ ] Database-backed tests cover block grouping, usage admission, duplicate delivery, and affected-parent resolution.
- [ ] Existing source-event skeleton behavior remains intact: full content is transient ingestion data and is not persisted in the event log.
- [ ] Backend clippy passes with warnings denied.
- [ ] Backend unit and database integration tests pass.
