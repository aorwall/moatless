-- The durable live tail of task events, and the one counter that orders it for
-- the whole deployment.
--
-- Two readers hold cursors over it: the SSE fan-out (one cursor per replica,
-- every event) and the event handler (one cursor for the fleet, each event
-- handled once).
--
-- Why not poll `task_events.id` directly: see `0104_task_listing_changes.sql`.
-- The `bigserial` visibility gap it describes applies here unchanged, and the
-- fix is the same — a counter allocated inside the writing transaction.

-- The deployment's event position counter. Exactly one row, and the `CHECK` is
-- what makes that true rather than conventional.
--
-- One counter for the whole deployment, not one per Task, because the SSE
-- firehose is cross-task: a replica holds a single cursor for every Task its
-- clients are watching.
--
-- The allocating statement holds this row until its transaction commits, so a
-- later position cannot become visible before an earlier one. Every event
-- publish serializes here; that is the cost of a cursor a reader cannot skip
-- past.
CREATE TABLE IF NOT EXISTS task_event_position (
    id       boolean PRIMARY KEY DEFAULT TRUE,
    position bigint  NOT NULL DEFAULT 0,
    CONSTRAINT task_event_position_is_one_row CHECK (id)
);

INSERT INTO task_event_position (id, position) VALUES (TRUE, 0)
    ON CONFLICT (id) DO NOTHING;

-- One record per published event, carrying the serialized envelope whole.
--
-- `event` is `text`, not `jsonb`: the SSE reader forwards these bytes to the
-- browser without parsing them, so storing them parsed would mean
-- re-serializing every row on every read to get back what the writer already
-- had.
--
-- `event_id` is UNIQUE because `IngressRelay` re-publishes any payload whose
-- delivery it cannot confirm: a re-publish allocates no new position, so no
-- client sees the event twice. A re-publish still re-arms dispatch — see
-- `task_event_dispatch_queue` — because the relay runs exactly when the
-- handlers may not have.
--
-- Deliberately no foreign key to `tasks`, for the reason spelled out in
-- `0104_task_listing_changes.sql`: a record has to outlive the Task it names,
-- since a Task going away is itself an event a client must still be told about.
--
-- Retention is `change_feed::events::prune_changes`, deleting below
-- `RETAINED_CHANGES` positions from the counter's head.
CREATE TABLE IF NOT EXISTS task_event_changes (
    position    bigint PRIMARY KEY,
    event_id    text NOT NULL UNIQUE,
    task_id     text NOT NULL,
    event       text NOT NULL,
    recorded_at timestamptz NOT NULL DEFAULT NOW()
);

-- What the fleet's event handler has not dispatched yet: one row per event,
-- deleted once a replica has run the handlers for it.
--
-- A queue rather than a single cursor row, because a cursor would serialize
-- every replica's handler work behind one lock. Claimed `FOR UPDATE SKIP
-- LOCKED`, so replicas dispatch disjoint batches in parallel.
--
-- `claimed_at` is a timestamp rather than a held row lock, so the claiming
-- transaction ends before the handlers run: a handler makes its own database
-- calls and awaits Adapter HTTP, and holding a transaction across that would
-- pin the xmin horizon against autovacuum for the whole batch. The cost is
-- that a replica dying mid-batch redelivers only once its claim ages past
-- `RECLAIM_AFTER`, rather than the instant its connection drops.
--
-- Ordering is deliberately not promised: two events of one Task can be handled
-- concurrently by different replicas. The handlers are written for that —
-- `persist_event` is `ON CONFLICT DO NOTHING` and `project_turn_state` is
-- `WHERE`-guarded.
--
-- A batch is deleted whether or not its handlers succeeded: a handler failure
-- is logged, never retried, so one poisoned event cannot wedge the queue. The
-- relay is what recovers an `agent.message` whose handler failed, by
-- re-publishing under the same `event_id` and re-arming the row here.
CREATE TABLE IF NOT EXISTS task_event_dispatch_queue (
    position   bigint PRIMARY KEY,
    claimed_at timestamptz
);

-- Serves the claim's `claimed_at IS NULL OR claimed_at < …` scan, so a claim
-- stays cheap when a stalled dispatcher has let the queue grow.
CREATE INDEX IF NOT EXISTS task_event_dispatch_queue_claimable
    ON task_event_dispatch_queue (claimed_at NULLS FIRST, position);
