-- The durable record that one Task's conversation changed, and the per-Task
-- counter that orders those records.
--
-- This is the sibling of `task_listing_changes` (0104) for a different question.
-- That feed answers "which Tasks changed since position N" for a sidebar, and
-- its counter is deliberately one row for the whole deployment because a client
-- holds a single position for the whole listing. Its own comment states the
-- cost that buys: changes advancing the position serialize against one another
-- there, "bounded by keeping agent output out of the feed entirely".
--
-- A conversation feed is agent output — a record per content block written. It
-- therefore cannot use that counter without putting every message write in the
-- deployment behind one row. So the counter here is per Task, which is also
-- what the reader wants: a client watching a conversation holds one position
-- for that conversation, and conversations are read one at a time.
--
-- Contention is bounded by construction. The writers for one Task are its
-- event-handler consumer, its gRPC ingress and whoever sends it a message —
-- already serialized against each other by the single `task_messages` row lock
-- they take on the way here.

-- One counter per Task. Absent until the Task's first change, which reads back
-- as position 0 — the same answer a Task with no conversation gives.
--
-- As in 0104, the allocating statement is `INSERT ... ON CONFLICT DO UPDATE
-- RETURNING`, which holds the row until its transaction commits. That is the
-- whole correctness argument: a second allocator for the same Task blocks until
-- the first commits, so a later position cannot become visible before an
-- earlier one, and commit order and position order are the same order. A
-- `bigserial` would not do — an id assigned at INSERT and made visible at
-- COMMIT can be read out of order, so a reader polling `> 99` can advance past
-- a 100 that had not committed yet and never see it, silently.
CREATE TABLE IF NOT EXISTS task_conversation_position (
    task_id  text   PRIMARY KEY,
    position bigint NOT NULL DEFAULT 0
);

-- One record per (transaction, subject) whose conversation-visible state moved.
--
-- `message_uuid` names *which* Message changed, or is NULL for a change to the
-- conversation itself — its Turn, its Sandbox, the session state derived from
-- both. It is a name, not a description: the reader re-reads the row at
-- delivery time and sends what it finds, exactly as 0104's does, so nothing
-- here has to describe what about the Message changed.
--
-- That naming is the difference from 0104, which needs none: a listing client
-- is sent one row per changed Task, whereas a conversation client is sent one
-- Message out of hundreds and re-reading the whole conversation to find it is
-- the cost this table exists to remove.
--
-- Deliberately no foreign key to `task_messages` or `tasks`. A record must
-- outlive what it names for the same reason 0104's must: the reader answers "no
-- such row" by ending the subscription, and a cascade would delete the record
-- that says so.
--
-- `(task_id, position)` is the primary key, so both readers are served by it
-- and neither needs a further index: the reader's `WHERE task_id = $1 AND
-- position >= $2 ORDER BY position`, and the prune's join to this Task's
-- counter.
--
-- Retention is by position from each Task's own head, as in 0104 and for the
-- same reason: `recorded_at` is not what the prune reads, because an age bound
-- is unbounded in rows and rows are what this costs. It is kept as the record
-- of when, and carries no index — this table takes an insert per Message
-- written, and an index nothing reads is paid for on every one of them.
CREATE TABLE IF NOT EXISTS task_conversation_changes (
    task_id      text   NOT NULL,
    position     bigint NOT NULL,
    message_uuid text,
    recorded_at  timestamptz NOT NULL DEFAULT NOW(),
    PRIMARY KEY (task_id, position)
);
