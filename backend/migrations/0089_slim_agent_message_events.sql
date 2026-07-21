-- 0089: Backfill fat `agent.message` rows in task_events to metadata skeletons
-- (ADR-0003).
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

DO $$
DECLARE
    slimmed integer;
BEGIN
    LOOP
        WITH batch AS (
            SELECT id
            FROM task_events
            WHERE event_type = 'agent'
              AND sub_type = 'message'
              AND payload -> 'message' ? 'content'
            ORDER BY id
            LIMIT 5000
        )
        UPDATE task_events t
        SET payload = jsonb_build_object(
            'message', jsonb_build_object(
                'role', COALESCE(t.payload -> 'message' ->> 'role', 'assistant')
            ),
            'uuid', COALESCE(t.payload -> 'uuid', 'null'::jsonb),
            'blockCount', CASE
                WHEN jsonb_typeof(t.payload -> 'message' -> 'content') = 'array'
                    THEN jsonb_array_length(t.payload -> 'message' -> 'content')
                ELSE 0
            END,
            'toolNames', CASE
                WHEN jsonb_typeof(t.payload -> 'message' -> 'content') = 'array' THEN
                    COALESCE((
                        SELECT jsonb_agg(elem ->> 'name')
                        FROM jsonb_array_elements(t.payload -> 'message' -> 'content') elem
                        WHERE elem ->> 'type' = 'tool_use'
                    ), '[]'::jsonb)
                ELSE '[]'::jsonb
            END
        )
        FROM batch b
        WHERE t.id = b.id;

        GET DIAGNOSTICS slimmed = ROW_COUNT;
        EXIT WHEN slimmed = 0;
    END LOOP;
END $$;
