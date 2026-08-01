-- 0089: Backfill fat `agent.message` rows in task_events to metadata skeletons
-- (the agent.message skeleton).
--
-- A Message's content lives once, in task_messages. The append-only task_events
-- log must not keep a second, permanent copy of it. This rewrites historical
-- `agent.message` rows — the log's largest payloads — to the skeleton
--   { message: { role }, uuid, blockCount, toolNames }
-- dropping their raw per-block content. The assembled content in task_messages
-- is untouched (this migration only touches task_events).
--
-- Batched by id range in a PL/pgSQL loop to avoid a long lock on a large,
-- unpruned table. Idempotent + gated: only rows whose payload is still fat (the
-- message still carries a `content` key) are rewritten, so a slimmed row is
-- never re-selected and re-running the migration is a no-op.
--
-- `payload` is `jsonb` on fresh installs (0013) but `text` on Drizzle-created
-- schemas (see 0013/0015 — CREATE TABLE IF NOT EXISTS keeps Drizzle's column
-- type). Reads cast `payload::jsonb` (a no-op when already jsonb, a parse when
-- text); the rebuilt skeleton is written back in the column's own type, appended
-- once from information_schema so the UPDATE type-checks on either schema.

DO $$
DECLARE
    slimmed integer;
    stmt    text;
BEGIN
    SELECT format($q$
        WITH batch AS (
            SELECT id
            FROM task_events
            WHERE type = 'agent'
              AND sub_type = 'message'
              AND (payload::jsonb) -> 'message' ? 'content'
            ORDER BY id
            LIMIT 5000
        )
        UPDATE task_events t
        SET payload = jsonb_build_object(
            'message', jsonb_build_object(
                'role', COALESCE((t.payload::jsonb) -> 'message' ->> 'role', 'assistant')
            ),
            'uuid', COALESCE((t.payload::jsonb) -> 'uuid', 'null'::jsonb),
            'blockCount', CASE
                WHEN jsonb_typeof((t.payload::jsonb) -> 'message' -> 'content') = 'array'
                    THEN jsonb_array_length((t.payload::jsonb) -> 'message' -> 'content')
                ELSE 0
            END,
            'toolNames', CASE
                WHEN jsonb_typeof((t.payload::jsonb) -> 'message' -> 'content') = 'array' THEN
                    COALESCE((
                        SELECT jsonb_agg(elem ->> 'name')
                        FROM jsonb_array_elements((t.payload::jsonb) -> 'message' -> 'content') elem
                        WHERE elem ->> 'type' = 'tool_use'
                    ), '[]'::jsonb)
                ELSE '[]'::jsonb
            END
        )%s
        FROM batch b
        WHERE t.id = b.id
    $q$, CASE WHEN data_type = 'jsonb' THEN '' ELSE '::text' END)
    INTO stmt
    FROM information_schema.columns
    WHERE table_name = 'task_events' AND column_name = 'payload';

    LOOP
        EXECUTE stmt;
        GET DIAGNOSTICS slimmed = ROW_COUNT;
        EXIT WHEN slimmed = 0;
    END LOOP;
END $$;
