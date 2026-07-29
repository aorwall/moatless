-- The durable record that a Task's listing-visible state changed, and the one
-- counter that orders those records for the whole deployment.
--
-- A sidebar listing is built from what a person Follows, and a client watching
-- one needs two things this backend could not previously answer: *which Tasks
-- changed since position N*, and a guarantee that asking again from N never
-- skips anything. `task_events` cannot answer either. Its `id` is a `bigserial`
-- assigned at INSERT and made visible at COMMIT, so two writers can take 100 and
-- 101, 101 can commit first, and a reader polling `id > 99` advances to 101 and
-- never sees 100 — silently, while reporting itself up to date. And most
-- mutations publish to the live channel and let a best-effort relay copy the
-- record afterwards, so a change can be durable nowhere at all.
--
-- These two tables fix both halves: the record is written by the same
-- transaction as the mutation it describes, and the position it carries is
-- allocated from the counter below *inside* that transaction.

-- The deployment's position counter. Exactly one row, and the `CHECK` is what
-- makes that true rather than conventional.
--
-- One counter for the whole deployment, not one per Task: the client holds a
-- single position for the entire listing and discards anything at or below it,
-- so a per-Task counter would produce many positions where there is room for
-- one.
--
-- The allocating statement is `UPDATE ... RETURNING`, which holds this row until
-- its transaction commits. That is the entire correctness argument: a second
-- allocator blocks until the first commits, so a later position cannot become
-- visible before an earlier one, and commit order and position order are the
-- same order. The cost is that changes advancing the position serialize against
-- one another here — accepted, and bounded by keeping agent output out of the
-- feed entirely.
CREATE TABLE IF NOT EXISTS task_listing_position (
    id       boolean PRIMARY KEY DEFAULT TRUE,
    position bigint  NOT NULL DEFAULT 0,
    CONSTRAINT task_listing_position_is_one_row CHECK (id)
);

INSERT INTO task_listing_position (id, position) VALUES (TRUE, 0)
    ON CONFLICT (id) DO NOTHING;

-- One record per (transaction, Task) whose listing-visible state changed.
--
-- It carries no description of *what* changed, because nothing reads one: a
-- watching client is sent the whole row, re-read at delivery time, never a
-- per-change difference. The record's only job is to name the Task and fix the
-- order.
--
-- `position` is the primary key, so the reader's `WHERE position > $1 ORDER BY
-- position` needs no further index.
--
-- Deliberately no foreign key to `tasks`. A record has to outlive the Task it
-- names, because a Task being deleted is itself a change a resuming client must
-- be told about, and `ON DELETE CASCADE` would erase precisely that record. A
-- Task that has gone away reads back as no row, which is what tells the delivery
-- path the listing entry is finished.
--
-- Retention is `task_listing::prune_changes`, deleting below `RETAINED_CHANGES`
-- positions from the counter's head — deliberately wider than the
-- `RESUME_WINDOW` a client may resume from, so a prune cannot delete a record a
-- resume has already promised. `recorded_at` is not what the prune reads: an
-- age bound is unbounded in rows, and rows are what this costs.
CREATE TABLE IF NOT EXISTS task_listing_changes (
    position    bigint PRIMARY KEY,
    task_id     text NOT NULL,
    recorded_at timestamptz NOT NULL DEFAULT NOW()
);
