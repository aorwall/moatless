-- Durable admission for provider webhook work that cannot finish on the HTTP
-- response path.
--
-- One row per (provider, kind, event_id) is written inside the webhook handler,
-- before the provider is acknowledged. From that moment the work is owned by
-- the backend: the supervised `adapter-work-worker` claims, executes, and marks
-- records terminal, and any replica can pick up work another replica admitted.
--
-- `event_id` is the *provider's* delivery identifier (Slack `event_id`,
-- Telegram `update_id`, GitHub `X-GitHub-Delivery`, Linear's webhook id), never
-- one the backend generates. That is what makes a re-delivered webhook a no-op:
-- the unique index below turns the second insert into a duplicate rather than a
-- second unit of work. A provider event that fans out into several independent
-- units of work distinguishes them by `kind`.
--
-- The table deliberately has no foreign keys. Admission runs on the provider's
-- response deadline and must never fail because a task, connection, or trigger
-- row is missing or concurrently deleted; the handler resolves those when it
-- executes and reports a permanent failure if they are gone.
CREATE TABLE IF NOT EXISTS adapter_work (
    id              text PRIMARY KEY,
    -- Provider label, used for metrics and operator queries ("slack",
    -- "telegram", "github", "linear").
    provider        text NOT NULL,
    -- Names the registered handler that executes this record.
    kind            text NOT NULL,
    -- The provider's own delivery identifier. Idempotency key, never generated
    -- by the backend.
    event_id        text NOT NULL,
    -- Everything the handler needs to redo the work from scratch after a
    -- restart. Self-contained by design: nothing in memory survives the crash
    -- this table exists to defend against.
    payload         jsonb NOT NULL,
    status          text NOT NULL DEFAULT 'pending',
    attempts        integer NOT NULL DEFAULT 0,
    max_attempts    integer NOT NULL,
    next_attempt_at timestamptz NOT NULL DEFAULT NOW(),
    -- Claim lease. A worker sets both when it takes a record; a claim whose
    -- `claimed_at` is older than the lease is recovered by any worker, which is
    -- how work interrupted by a crash comes back.
    claimed_at      timestamptz,
    claimed_by      text,
    -- Last failure, retained on terminal records so an abandoned webhook is
    -- distinguishable from one that never arrived.
    last_error      text,
    admitted_at     timestamptz NOT NULL DEFAULT NOW(),
    updated_at      timestamptz NOT NULL DEFAULT NOW(),
    CONSTRAINT adapter_work_status_check
        CHECK (status IN ('pending', 'claimed', 'succeeded', 'failed'))
);

-- Idempotent admission. A duplicate delivery of the same provider event for the
-- same work kind conflicts here and is acknowledged without creating new work.
CREATE UNIQUE INDEX IF NOT EXISTS adapter_work_event_idx
    ON adapter_work (provider, kind, event_id);

-- Admission counts the unfinished backlog on the provider's response deadline,
-- so that count must never touch finished rows. This partial index matches the
-- capacity predicate exactly and holds at most `backlog_capacity` entries,
-- which keeps the check an index-only scan over a small, bounded set.
CREATE INDEX IF NOT EXISTS adapter_work_unfinished_idx
    ON adapter_work (id)
    WHERE status IN ('pending', 'claimed');

-- The worker's claim scan: due pending records, oldest first.
CREATE INDEX IF NOT EXISTS adapter_work_due_idx
    ON adapter_work (next_attempt_at)
    WHERE status = 'pending';

-- Stale-claim recovery scan.
CREATE INDEX IF NOT EXISTS adapter_work_claimed_idx
    ON adapter_work (claimed_at)
    WHERE status = 'claimed';

-- Retention pruning of finished records. Successful records are kept long
-- enough to cover every provider's re-delivery window, so a late duplicate
-- still deduplicates; terminal failures are kept longer because they are the
-- operator's record of abandoned work.
CREATE INDEX IF NOT EXISTS adapter_work_finished_idx
    ON adapter_work (updated_at)
    WHERE status IN ('succeeded', 'failed');
