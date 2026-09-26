-- Agent type is renamed Agent profile: the column names a catalog entry
-- (harness, transport, default model), never a harness.
ALTER TABLE tasks        RENAME COLUMN agent_type TO agent_profile;
ALTER TABLE task_turns   RENAME COLUMN agent_type TO agent_profile;
ALTER TABLE sandboxes    RENAME COLUMN agent_type TO agent_profile;
ALTER TABLE task_events  RENAME COLUMN agent_type TO agent_profile;
ALTER TABLE loop_configs RENAME COLUMN agent_type TO agent_profile;

-- A stored `StreamEvent` payload carries the key at its top level. The column
-- is `text` in older deployments and `jsonb` in newer ones; the jsonb result
-- assigns to either. `as_jsonb` answers NULL for a text payload that is not
-- JSON, which a bare cast would abort the migration on; `pg_input_is_valid`
-- would need Postgres 16.
CREATE FUNCTION pg_temp.as_jsonb(payload text) RETURNS jsonb
    LANGUAGE plpgsql IMMUTABLE AS $$
BEGIN
    RETURN payload::jsonb;
EXCEPTION WHEN others THEN
    RETURN NULL;
END
$$;

UPDATE task_events
   SET payload = (pg_temp.as_jsonb(payload::text) - 'agentType')
                 || jsonb_build_object('agentProfile',
                                       pg_temp.as_jsonb(payload::text) -> 'agentType')
 WHERE payload::text LIKE '%"agentType"%'
   AND pg_temp.as_jsonb(payload::text) ? 'agentType';
