-- Transactional ingress outbox for sandbox-published event payloads (ADR-0006).
--
-- The backend acknowledges a sandbox event only after the complete dispatch
-- payload has reached the durability that event kind requires. Most kinds get
-- that from `task_events` (full payload) or `task_turns` (committed state).
-- `agent.message` cannot: ADR-0003 makes its `task_events` row a metadata
-- skeleton, so the full content has no durable home between the gRPC ack and
-- the consumer's `task_messages` commit.
--
-- This table is that home. It is a **transient spool**, not an event log:
-- one row exists only from admission until the consumer commits the message,
-- which deletes it. `task_messages` remains the canonical message record and
-- `task_events` still stores only the ADR-0003 skeleton — this is deliberately
-- NOT the permanent `task_events` copy ADR-0003 removed.
--
-- Retry: the relay claims due rows (`next_attempt_at <= now`, `FOR UPDATE SKIP
-- LOCKED`), reschedules them with exponential backoff in the same statement, and
-- republishes them to the consumer path. Only a confirmed `task_messages` commit
-- deletes a row, so a lost publish converges instead of losing content.
--
-- Rows die with their Task (`ON DELETE CASCADE`); nothing else prunes them,
-- because a row that still exists is by definition content that never landed.
CREATE TABLE IF NOT EXISTS event_ingress_outbox (
    event_id        text PRIMARY KEY,
    task_id         text NOT NULL REFERENCES tasks(task_id) ON DELETE CASCADE,
    event_type      text NOT NULL,
    sub_type        text NOT NULL,
    turn_number     integer,
    payload         jsonb NOT NULL,
    timestamp       text NOT NULL,
    admitted_at     timestamptz NOT NULL DEFAULT NOW(),
    attempts        integer NOT NULL DEFAULT 0,
    next_attempt_at timestamptz NOT NULL
);

-- The relay's only scan: due records, oldest deadline first.
CREATE INDEX IF NOT EXISTS event_ingress_outbox_due_idx
    ON event_ingress_outbox (next_attempt_at);
