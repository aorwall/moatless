-- Every message addressed to the agent. `origin` decides which sender and
-- channel columns are set, and the CHECK below states each origin's exact
-- shape. `unknown` exists only for delivered history whose sender did not
-- record.
CREATE TABLE IF NOT EXISTS task_inbox (
    id                    uuid PRIMARY KEY,
    task_id               text NOT NULL REFERENCES tasks (task_id) ON DELETE CASCADE,
    created_at            timestamptz NOT NULL DEFAULT now(),

    text                  text NOT NULL,
    file_ids              text[] NOT NULL DEFAULT '{}',
    command_name          text,
    -- Model, mode, effort, skills, resume, start-fresh, contexts and a tool
    -- response's structured answer: what the turn runner reads beside `text`.
    delivery_options      jsonb,
    answers_tool_use_id   text,

    origin                text NOT NULL
                          CHECK (origin IN ('user', 'task', 'adapter', 'system', 'sandbox', 'unknown')),

    sent_by               text,
    source_task_id        text,
    external_sender_id    text,
    external_author       text,

    adapter               text,
    external_message_id   text,
    external_subject_key  text,
    external_source_name  text,

    status                text NOT NULL DEFAULT 'pending'
                          CHECK (status IN ('pending', 'delivered', 'cancelled')),
    delivered_at          timestamptz,
    run_id                text,
    turn_number           integer,
    -- The agent session the delivery ran in; recovery resumes it.
    session_id            text,

    CONSTRAINT task_inbox_sender_check CHECK (CASE origin
      WHEN 'user'    THEN sent_by IS NOT NULL
                          AND num_nonnulls(source_task_id, adapter, external_message_id,
                                           external_sender_id, external_author) = 0
      WHEN 'task'    THEN source_task_id IS NOT NULL
                          AND num_nonnulls(adapter, external_message_id,
                                           external_sender_id, external_author) = 0
      WHEN 'adapter' THEN adapter IS NOT NULL AND external_message_id IS NOT NULL
                          AND source_task_id IS NULL
      WHEN 'system'  THEN num_nonnulls(sent_by, source_task_id, adapter, external_message_id,
                                       external_sender_id, external_author) = 0
      WHEN 'sandbox' THEN num_nonnulls(sent_by, source_task_id, adapter, external_message_id,
                                       external_sender_id, external_author) = 0
      WHEN 'unknown' THEN status = 'delivered'
    END),
    CONSTRAINT task_inbox_channel_check
        CHECK (adapter IS NOT NULL OR num_nonnulls(external_subject_key, external_source_name) = 0),
    CONSTRAINT task_inbox_delivered_at_check
        CHECK ((status = 'delivered') = (delivered_at IS NOT NULL)),
    CONSTRAINT task_inbox_stamp_check
        CHECK (status = 'delivered' OR num_nonnulls(run_id, turn_number, session_id) = 0)
);

CREATE UNIQUE INDEX IF NOT EXISTS task_inbox_external_message_idx
    ON task_inbox (task_id, adapter, external_message_id) WHERE external_message_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS task_inbox_pending_idx
    ON task_inbox (task_id, created_at) WHERE status = 'pending';
CREATE INDEX IF NOT EXISTS task_inbox_sent_by_idx ON task_inbox (sent_by);
CREATE INDEX IF NOT EXISTS task_inbox_turn_idx ON task_inbox (task_id, turn_number);

-- backfill:begin
-- The inbox's id is a uuid, so a queued Message whose id is not one takes a
-- stable uuid here, and so does every row naming it: delivery stamps the
-- Message by the item's id.
CREATE TEMP TABLE task_inbox_renamed ON COMMIT DROP AS
SELECT m.task_id, m.uuid AS old_uuid,
       md5('task_inbox:' || m.task_id || ':' || m.uuid)::uuid::text AS new_uuid
  FROM task_messages m
 WHERE m.message_type = 'user'
   AND m.agent_session_id IS NULL
   AND m.uuid !~* '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$';
UPDATE task_messages m SET uuid = r.new_uuid
  FROM task_inbox_renamed r WHERE m.task_id = r.task_id AND m.uuid = r.old_uuid;
UPDATE task_activities a SET message_uuid = r.new_uuid
  FROM task_inbox_renamed r WHERE a.task_id = r.task_id AND a.message_uuid = r.old_uuid;
UPDATE task_message_usage u SET message_uuid = r.new_uuid
  FROM task_inbox_renamed r WHERE u.task_id = r.task_id AND u.message_uuid = r.old_uuid;
UPDATE task_conversation_changes c SET message_uuid = r.new_uuid
  FROM task_inbox_renamed r WHERE c.task_id = r.task_id AND c.message_uuid = r.old_uuid;
UPDATE feedback f SET message_uuid = r.new_uuid
  FROM task_inbox_renamed r WHERE f.task_id = r.task_id AND f.message_uuid = r.old_uuid;

-- Queued Messages move to the inbox under their own id, and so do the
-- delivered ones of a turn still open, which recovery replays from the inbox.
-- A row with no recorded sender is `system`, which the CHECK allows either way.
WITH moved AS (
    SELECT m.*,
           CASE
               WHEN m.user_type = 'harness' THEN 'sandbox'
               WHEN m.source_task_id IS NOT NULL THEN 'task'
               WHEN m.external_adapter IS NOT NULL AND m.external_event_id IS NOT NULL THEN 'adapter'
               WHEN m.sent_by IS NOT NULL THEN 'user'
               ELSE 'system'
           END AS origin,
           t.run_id AS turn_run_id
      FROM task_messages m
      LEFT JOIN task_turns t
        ON t.task_id = m.task_id AND t.turn_number = m.turn_number
     WHERE m.message_type = 'user'
       AND m.uuid ~* '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
       AND (m.agent_session_id IS NULL
            OR (m.turn_number IS NOT NULL AND t.status IN ('active', 'interrupted')))
)
INSERT INTO task_inbox
    (id, task_id, created_at, text, file_ids, delivery_options, answers_tool_use_id, origin,
     sent_by, source_task_id, external_sender_id, external_author,
     adapter, external_message_id, external_subject_key, external_source_name,
     status, delivered_at, run_id, session_id, turn_number)
SELECT p.uuid::uuid,
       p.task_id,
       COALESCE(p.created_at, now()),
       COALESCE(p.message_text, ''),
       COALESCE(ARRAY(SELECT jsonb_array_elements_text(p.attachments -> 'file_ids')), '{}'),
       jsonb_strip_nulls(jsonb_build_object(
           'model', p.model,
           'agentMode', p.agent_mode,
           'effort', p.effort,
           'skills', to_jsonb(p.skills),
           'resume', p.resume,
           'startFresh', p.start_fresh,
           'toolResponse', p.tool_response,
           'contexts', (SELECT jsonb_agg(jsonb_build_object(
                                   'type', c.type, 'referenceId', c.reference_id, 'data', c.data)
                                   ORDER BY c.id)
                          FROM task_message_contexts c
                         WHERE c.task_message_id = p.id))),
       p.tool_response ->> 'toolUseId',
       p.origin,
       CASE WHEN p.origin IN ('user', 'task', 'adapter') THEN p.sent_by END,
       CASE WHEN p.origin = 'task' THEN p.source_task_id END,
       CASE WHEN p.origin = 'adapter' THEN p.external_sender_id END,
       CASE WHEN p.origin = 'adapter' THEN p.external_author END,
       CASE WHEN p.origin = 'adapter' THEN p.external_adapter END,
       CASE WHEN p.origin = 'adapter' THEN p.external_event_id END,
       CASE WHEN p.origin = 'adapter' THEN p.external_subject_key END,
       CASE WHEN p.origin = 'adapter' THEN p.external_source_name END,
       CASE WHEN p.agent_session_id IS NULL THEN 'pending' ELSE 'delivered' END,
       CASE WHEN p.agent_session_id IS NOT NULL
            THEN COALESCE(p.sent_at, p.created_at, now()) END,
       CASE WHEN p.agent_session_id IS NOT NULL THEN p.turn_run_id END,
       p.agent_session_id,
       CASE WHEN p.agent_session_id IS NOT NULL THEN p.turn_number END
  FROM moved p
    ON CONFLICT DO NOTHING;
-- backfill:end

-- A turn's trigger is recorded only once it is decided.
UPDATE task_turns SET trigger_decided = true
 WHERE triggered_by_task_id IS NOT NULL AND NOT trigger_decided;
ALTER TABLE task_turns
    DROP CONSTRAINT IF EXISTS task_turns_trigger_decided_check,
    ADD CONSTRAINT task_turns_trigger_decided_check
        CHECK (trigger_decided OR triggered_by_task_id IS NULL);
