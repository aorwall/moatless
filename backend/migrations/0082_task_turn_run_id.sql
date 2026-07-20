-- task_turns.run_id: the sandbox agent run a turn belongs to.
--
-- Turn numbers are allocated at acknowledgment, keyed by the run id the sandbox
-- mints per agent run. Both `taskTurn.active` and the ack carry that id, and
-- whichever reaches the backend first allocates the turn while the other joins
-- it — the two travel on unsynchronized channels (the event outbox retries with
-- backoff up to 30s, the ack is a direct unary call), so neither arrival order
-- may decide turn membership.
--
-- Nullable: rows written before this column existed have no run to key on.
ALTER TABLE task_turns ADD COLUMN IF NOT EXISTS run_id text;

-- The allocator looks a run up by this key and inserts under it, so the
-- uniqueness is what makes concurrent allocation for one run converge on a
-- single turn. Partial, since legacy rows share a NULL run_id.
CREATE UNIQUE INDEX IF NOT EXISTS idx_task_turns_run
    ON task_turns (task_id, run_id)
    WHERE run_id IS NOT NULL;

-- Recovery is per turn, not per message: a turn can hold several user messages
-- (everything the sandbox coalesced into one agent run), and bumping the count
-- on one of them would undercount the batch.
ALTER TABLE task_turns
    ADD COLUMN IF NOT EXISTS recovery_attempts integer NOT NULL DEFAULT 0;
