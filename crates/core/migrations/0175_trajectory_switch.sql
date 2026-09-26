-- A switch session: its file starts with the other harness's lines rendered
-- into this harness's format. `transcoded_from_event_id` is the other
-- harness's tip they were rendered from; lineage only, `chain` never follows
-- it. `own_from_seq` is the first seq the CLI itself wrote, set by the
-- session's first append, so it may be NULL only while the session has no
-- event of its own.
ALTER TABLE trajectory_sessions
    ADD COLUMN IF NOT EXISTS transcoded_from_event_id uuid REFERENCES trajectory_events (id),
    ADD COLUMN IF NOT EXISTS own_from_seq bigint,
    ADD CONSTRAINT trajectory_sessions_own_from_seq_check
        CHECK (own_from_seq IS NULL OR transcoded_from_event_id IS NOT NULL),
    DROP CONSTRAINT IF EXISTS trajectory_sessions_subagent_lineage_check,
    ADD CONSTRAINT trajectory_sessions_subagent_lineage_check
        CHECK (parent_event_id IS NULL
               OR num_nonnulls(transcoded_from_event_id, forked_from_event_id) = 0);

CREATE INDEX IF NOT EXISTS trajectory_sessions_task_transcoded_idx
    ON trajectory_sessions (task_id, transcoded_from_event_id);

-- `forked_from_event_id` continues a file of the same format, so it must name
-- an event of a session with the same harness. It spans two rows, so it is a
-- trigger rather than a CHECK.
CREATE OR REPLACE FUNCTION trajectory_sessions_fork_same_harness() RETURNS trigger AS $$
BEGIN
    IF NEW.forked_from_event_id IS NOT NULL AND NOT EXISTS (
        SELECT 1
          FROM trajectory_events e
          JOIN trajectory_sessions s ON s.id = e.session_id
         WHERE e.id = NEW.forked_from_event_id AND s.harness = NEW.harness
    ) THEN
        RAISE EXCEPTION 'trajectory session % forks an event of another harness', NEW.id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trajectory_sessions_fork_same_harness ON trajectory_sessions;
CREATE CONSTRAINT TRIGGER trajectory_sessions_fork_same_harness
    AFTER INSERT OR UPDATE OF forked_from_event_id, harness ON trajectory_sessions
    FOR EACH ROW EXECUTE FUNCTION trajectory_sessions_fork_same_harness();

-- The cut a fork or rewind continues from, as one event; its session and
-- harness are read through it. NULL for a row written before this column,
-- which the read path resolves from the anchors beside it, and for a cut with
-- no recorded agent session.
ALTER TABLE task_forks
    ADD COLUMN IF NOT EXISTS source_event_id uuid REFERENCES trajectory_events (id);

-- Lineage order. An event id is a UUIDv7 from the writing process's clock, so
-- ids from two backends need not follow the order events were written. `ord`
-- comes from one sequence, and an event is inserted only after its parent and
-- any tip it was transcoded from. The backfill orders existing rows by the
-- database's clock rather than by id; the running max keeps a session's own
-- lines in seq order when an append waited on the row lock of an earlier one.
ALTER TABLE trajectory_events ADD COLUMN IF NOT EXISTS ord bigint;
UPDATE trajectory_events e
   SET ord = o.n
  FROM (SELECT id,
               row_number() OVER (ORDER BY written, seq, id) AS n
          FROM (SELECT id, seq,
                       max(created_at) OVER (PARTITION BY session_id ORDER BY seq) AS written
                  FROM trajectory_events) w) o
 WHERE e.id = o.id AND e.ord IS NULL;
CREATE SEQUENCE IF NOT EXISTS trajectory_events_ord_seq OWNED BY trajectory_events.ord;
SELECT setval('trajectory_events_ord_seq',
              COALESCE((SELECT max(ord) FROM trajectory_events), 0) + 1, false);
ALTER TABLE trajectory_events
    ALTER COLUMN ord SET DEFAULT nextval('trajectory_events_ord_seq'),
    ALTER COLUMN ord SET NOT NULL;
