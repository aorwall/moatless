-- The conversation read model. The Projector writes every row from one source:
-- an inbox item, a Trajectory line or a task event. A `frozen` row is history
-- copied once from `task_messages`/`task_activities` and names no source.
CREATE TABLE IF NOT EXISTS conversation_messages (
    uuid                 uuid PRIMARY KEY,
    task_id              text NOT NULL,
    -- The top-level stream's position, or the position within the subagent
    -- transcript `parent_tool_use_id` names.
    position             bigint NOT NULL,
    author               text NOT NULL CHECK (author IN ('user', 'agent', 'system')),
    kind                 text NOT NULL CHECK (kind IN ('text', 'error', 'turn_result')),
    text                 text,
    text_parts           text[],
    -- The spawning `Agent` call's tool-use id, for a subagent's Message.
    parent_tool_use_id   text,
    turn_number          integer,
    harness              text CHECK (harness IN ('claude-code', 'codex')),
    model                text,
    usage                jsonb,
    inbox_id             uuid REFERENCES task_inbox (id) ON DELETE CASCADE,
    trajectory_event_id  uuid REFERENCES trajectory_events (id),
    task_event_id        text,
    frozen               boolean NOT NULL DEFAULT false,
    -- A frozen row's `task_messages` columns, text excepted.
    legacy               jsonb,
    created_at           timestamptz NOT NULL DEFAULT now(),
    CHECK (frozen OR num_nonnulls(inbox_id, trajectory_event_id, task_event_id) = 1),
    CHECK (frozen OR CASE
      WHEN author = 'user'  THEN kind = 'text' AND inbox_id IS NOT NULL
      WHEN author = 'agent' THEN kind = 'text' AND inbox_id IS NULL
                                 AND (trajectory_event_id IS NOT NULL) = (harness IS NOT NULL)
      ELSE kind IN ('error', 'turn_result') AND task_event_id IS NOT NULL
    END),
    CHECK (frozen OR usage IS NULL OR kind = 'turn_result' OR author = 'agent'),
    CHECK (frozen = (legacy IS NOT NULL))
);
CREATE UNIQUE INDEX IF NOT EXISTS conversation_messages_task_position_idx
    ON conversation_messages (task_id, position) WHERE parent_tool_use_id IS NULL;
CREATE INDEX IF NOT EXISTS conversation_messages_parent_idx
    ON conversation_messages (task_id, parent_tool_use_id, position)
    WHERE parent_tool_use_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS conversation_messages_inbox_idx ON conversation_messages (inbox_id);
CREATE INDEX IF NOT EXISTS conversation_messages_task_event_idx
    ON conversation_messages (task_event_id) WHERE task_event_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS conversation_messages_event_idx
    ON conversation_messages (trajectory_event_id);
CREATE INDEX IF NOT EXISTS conversation_messages_task_turn_idx
    ON conversation_messages (task_id, turn_number);
-- The shell listing's per-conversation lookups: the person's newest Message,
-- and the model the newest turn's agent ran.
CREATE INDEX IF NOT EXISTS conversation_messages_latest_user_idx
    ON conversation_messages (task_id, created_at DESC) WHERE author = 'user';
CREATE INDEX IF NOT EXISTS conversation_messages_turn_agent_model_idx
    ON conversation_messages (task_id, turn_number, position DESC) INCLUDE (model)
    WHERE author = 'agent' AND model IS NOT NULL;

-- An agent Message from a task event is a reply sent through a binding, and
-- nothing else. It spans two rows, so it is a trigger rather than a CHECK.
CREATE OR REPLACE FUNCTION conversation_messages_agent_event() RETURNS trigger AS $$
BEGIN
    IF NEW.author = 'agent' AND NEW.task_event_id IS NOT NULL AND NOT NEW.frozen
       AND NOT EXISTS (
        SELECT 1 FROM task_events
         WHERE event_id = NEW.task_event_id AND type = 'task' AND sub_type = 'binding_reply_sent'
    ) THEN
        RAISE EXCEPTION 'agent Message % names task event % that is not a binding reply',
            NEW.uuid, NEW.task_event_id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS conversation_messages_agent_event ON conversation_messages;
CREATE TRIGGER conversation_messages_agent_event
    BEFORE INSERT OR UPDATE ON conversation_messages
    FOR EACH ROW EXECUTE FUNCTION conversation_messages_agent_event();

CREATE TABLE IF NOT EXISTS conversation_activities (
    activity_id          uuid PRIMARY KEY,
    task_id              text NOT NULL,
    position             bigint NOT NULL,
    -- The block's index within its Message; orders rows sharing a position.
    ordinal              integer NOT NULL DEFAULT 0,
    message_uuid         uuid,
    parent_tool_use_id   text,
    kind                 text NOT NULL CHECK (kind IN (
                             'tool_call', 'thinking', 'user_input_requested', 'plan_updated',
                             'user_input_resolved', 'sandbox_status', 'sandbox_warning',
                             'sandbox_error', 'context_window_updated')),
    status               text NOT NULL CHECK (status IN ('in_progress', 'completed', 'failed')),
    tool_use_id          text,
    summary              text NOT NULL,
    payload              jsonb NOT NULL DEFAULT '{}'::jsonb,
    detail               text,
    is_error             boolean NOT NULL DEFAULT false,
    trajectory_event_id  uuid REFERENCES trajectory_events (id),  -- the call
    result_event_id      uuid REFERENCES trajectory_events (id),  -- the result
    inbox_id             uuid REFERENCES task_inbox (id) ON DELETE CASCADE,
    task_event_id        text,
    frozen               boolean NOT NULL DEFAULT false,
    created_at           timestamptz NOT NULL DEFAULT now(),
    CHECK (frozen OR CASE
      WHEN kind = 'tool_call' THEN trajectory_event_id IS NOT NULL AND tool_use_id IS NOT NULL
                                   AND num_nonnulls(inbox_id, task_event_id) = 0
      WHEN kind IN ('thinking', 'user_input_requested', 'plan_updated')
                              THEN trajectory_event_id IS NOT NULL
                                   AND num_nonnulls(tool_use_id, result_event_id, inbox_id,
                                                    task_event_id) = 0
      WHEN kind = 'user_input_resolved'
                              THEN inbox_id IS NOT NULL
                                   AND num_nonnulls(trajectory_event_id, result_event_id,
                                                    tool_use_id, task_event_id) = 0
      ELSE task_event_id IS NOT NULL
           AND num_nonnulls(trajectory_event_id, result_event_id, tool_use_id, inbox_id) = 0
    END),
    CHECK (result_event_id IS NULL OR status <> 'in_progress')
);
CREATE INDEX IF NOT EXISTS conversation_activities_task_idx
    ON conversation_activities (task_id, position);
CREATE UNIQUE INDEX IF NOT EXISTS conversation_activities_tool_use_idx
    ON conversation_activities (task_id, tool_use_id) WHERE tool_use_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS conversation_activities_task_event_idx
    ON conversation_activities (task_event_id) WHERE task_event_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS conversation_activities_message_idx
    ON conversation_activities (task_id, message_uuid) WHERE message_uuid IS NOT NULL;
CREATE INDEX IF NOT EXISTS conversation_activities_parent_idx
    ON conversation_activities (task_id, parent_tool_use_id)
    WHERE parent_tool_use_id IS NOT NULL;

-- One model response's token counts, admitted once. A live row is keyed by
-- the line that reported it; a frozen row keeps its mapped Message id and the
-- subagent it counted for.
CREATE TABLE IF NOT EXISTS conversation_usage (
    trajectory_event_id          uuid REFERENCES trajectory_events (id),
    message_uuid                 uuid UNIQUE,
    task_id                      text NOT NULL,
    parent_tool_use_id           text,
    input_tokens                 integer NOT NULL DEFAULT 0,
    output_tokens                integer NOT NULL DEFAULT 0,
    cache_creation_input_tokens  integer NOT NULL DEFAULT 0,
    cache_read_input_tokens      integer NOT NULL DEFAULT 0,
    CHECK (num_nonnulls(trajectory_event_id, message_uuid) = 1),
    CHECK (trajectory_event_id IS NULL OR parent_tool_use_id IS NULL),
    -- Per Task: a fork admits the usage of the lines it inherits.
    UNIQUE (task_id, trajectory_event_id)
);
CREATE INDEX IF NOT EXISTS conversation_usage_task_idx ON conversation_usage (task_id);

-- When a Task's history froze, and its turn then still open: a later source
-- may settle a frozen row only if that turn wrote it, at or after `open_since`.
CREATE TABLE IF NOT EXISTS conversation_frozen (
    task_id     text PRIMARY KEY,
    frozen_at   timestamptz NOT NULL DEFAULT now(),
    open_turn   integer,
    open_since  timestamptz
);

-- An item a rewind cut out of its Task's conversation stays `rewound` while a
-- fork's inherited Message reads it, keeping its turn and delivery.
ALTER TABLE task_inbox
    DROP CONSTRAINT IF EXISTS task_inbox_status_check,
    DROP CONSTRAINT IF EXISTS task_inbox_delivered_at_check,
    DROP CONSTRAINT IF EXISTS task_inbox_stamp_check,
    DROP CONSTRAINT IF EXISTS task_inbox_sender_check,
    ADD COLUMN IF NOT EXISTS rewound_at timestamptz;
ALTER TABLE task_inbox
    ADD CONSTRAINT task_inbox_status_check
        CHECK (status IN ('pending', 'delivered', 'cancelled', 'rewound')),
    ADD CONSTRAINT task_inbox_delivered_at_check
        CHECK ((status IN ('delivered', 'rewound')) = (delivered_at IS NOT NULL)),
    ADD CONSTRAINT task_inbox_stamp_check
        CHECK (status IN ('delivered', 'rewound') OR num_nonnulls(run_id, turn_number, session_id) = 0),
    ADD CONSTRAINT task_inbox_rewound_at_check
        CHECK ((status = 'rewound') = (rewound_at IS NOT NULL)),
    ADD CONSTRAINT task_inbox_sender_check CHECK (CASE origin
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
      WHEN 'unknown' THEN status IN ('delivered', 'rewound')
    END);
-- The shell listing's newest queued item.
CREATE INDEX IF NOT EXISTS task_inbox_latest_queued_idx
    ON task_inbox (task_id, created_at DESC) WHERE status IN ('pending', 'delivered');
-- A fork's inherited Message reads its ancestor's item, so the item outlives
-- the ancestor; deleting a Task deletes the items no other Task reads.
ALTER TABLE task_inbox DROP CONSTRAINT IF EXISTS task_inbox_task_id_fkey;

-- freeze:begin
-- A legacy id that is not a uuid becomes an md5 name-based one, its version
-- nibble 3, so it never equals a Projector id (version 5).
CREATE FUNCTION pg_temp.legacy_uuid(id text) RETURNS uuid LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE
        WHEN id ~* '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$' THEN id::uuid
        ELSE (SELECT (substr(h, 1, 12) || '3' || substr(h, 14, 3)
                      || to_hex((('x' || substr(h, 17, 1))::bit(4)::int & 3) | 8)
                      || substr(h, 18, 15))::uuid
                FROM md5('legacy:' || id) AS h)
    END
$$;

INSERT INTO conversation_frozen (task_id, open_turn, open_since)
SELECT f.task_id, t.turn_number, t.started_at
  FROM (SELECT task_id FROM task_messages UNION SELECT task_id FROM task_activities) f
  LEFT JOIN LATERAL (
      SELECT o.turn_number, o.started_at FROM task_turns o
       WHERE o.task_id = f.task_id AND o.status IN ('active', 'interrupted')
       ORDER BY o.turn_number DESC
       LIMIT 1) t ON true
    ON CONFLICT DO NOTHING;

-- Every user row gets an inbox item, so a frozen user Message keeps its origin
-- and a fork can inherit it. Items 0176 already moved keep theirs.
INSERT INTO task_inbox
    (id, task_id, created_at, text, file_ids, delivery_options, answers_tool_use_id, origin,
     sent_by, source_task_id, external_sender_id, external_author,
     adapter, external_message_id, external_subject_key, external_source_name,
     status, delivered_at, run_id, session_id, turn_number)
SELECT pg_temp.legacy_uuid(p.uuid),
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
       'delivered',
       COALESCE(p.sent_at, p.created_at, now()),
       p.turn_run_id,
       p.agent_session_id,
       p.turn_number
  FROM (SELECT m.*,
               CASE
                   WHEN m.user_type = 'harness' THEN 'sandbox'
                   WHEN m.source_task_id IS NOT NULL THEN 'task'
                   WHEN m.external_adapter IS NOT NULL AND m.external_event_id IS NOT NULL
                       THEN 'adapter'
                   WHEN m.sent_by IS NOT NULL THEN 'user'
                   ELSE 'system'
               END AS origin,
               t.run_id AS turn_run_id
          FROM task_messages m
          LEFT JOIN task_turns t ON t.task_id = m.task_id AND t.turn_number = m.turn_number
         WHERE m.message_type = 'user') p
    ON CONFLICT DO NOTHING;

-- A reply the Task posted through a binding is an event it caused, which the
-- echo check reads from `task_outbound_events`.
INSERT INTO task_outbound_events (external_event_id, task_id, provider, source_name, created_at)
SELECT m.external_event_id, m.task_id, m.external_adapter, m.external_source_name, m.created_at
  FROM task_messages m
 WHERE m.message_type <> 'user'
   AND m.external_event_id IS NOT NULL AND m.external_adapter IS NOT NULL
    ON CONFLICT DO NOTHING;

-- Positions 1..n per stream (the Task's own, and each subagent transcript):
-- Messages in their order, each followed by its activities in theirs.
CREATE TEMP TABLE frozen_order ON COMMIT DROP AS
WITH rows AS (
    SELECT m.task_id, 'message' AS what, m.uuid AS legacy_id, m.parent_uuid AS parent,
           m.created_at AS at, m.id::bigint AS anchor, 0 AS sub, 0::bigint AS k1, 0 AS k2
      FROM task_messages m
    UNION ALL
    SELECT a.task_id, 'activity', a.activity_id, a.parent_uuid,
           COALESCE(m.created_at, a.created_at), COALESCE(m.id, 0)::bigint, 1,
           a.position, a.ordinal
      FROM task_activities a
      LEFT JOIN task_messages m ON m.uuid = a.message_uuid AND m.task_id = a.task_id
)
SELECT task_id, what, legacy_id, parent,
       row_number() OVER (PARTITION BY task_id, parent
                          ORDER BY at, anchor, sub, k1, k2, legacy_id) AS position
  FROM rows;
CREATE INDEX ON frozen_order (what, legacy_id);

INSERT INTO conversation_messages
    (uuid, task_id, position, author, kind, text, text_parts, parent_tool_use_id, turn_number,
     model, usage, inbox_id, frozen, legacy, created_at)
SELECT pg_temp.legacy_uuid(m.uuid), m.task_id, o.position,
       CASE m.message_type WHEN 'user' THEN 'user' WHEN 'assistant' THEN 'agent' ELSE 'system' END,
       CASE m.message_type WHEN 'error' THEN 'error' WHEN 'result' THEN 'turn_result' ELSE 'text' END,
       m.message_text, m.message_text_parts, m.parent_uuid, m.turn_number,
       m.assistant_message_model, m.assistant_usage,
       (SELECT i.id FROM task_inbox i
         WHERE m.message_type = 'user' AND i.id = pg_temp.legacy_uuid(m.uuid)),
       true,
       to_jsonb(m) - 'message_text' - 'message_text_parts' - 'message_content' - 'id',
       m.created_at
  FROM task_messages m
  JOIN frozen_order o ON o.what = 'message' AND o.legacy_id = m.uuid;

INSERT INTO conversation_activities
    (activity_id, task_id, position, ordinal, message_uuid, parent_tool_use_id, kind, status,
     tool_use_id, summary, payload, detail, is_error, frozen, created_at)
SELECT pg_temp.legacy_uuid(a.activity_id), a.task_id, o.position, a.ordinal,
       pg_temp.legacy_uuid(a.message_uuid), a.parent_uuid, a.kind, a.status,
       a.tool_use_id, a.summary, a.payload, a.detail, a.is_error, true, a.created_at
  FROM task_activities a
  JOIN frozen_order o ON o.what = 'activity' AND o.legacy_id = a.activity_id;

INSERT INTO conversation_usage
    (message_uuid, task_id, parent_tool_use_id, input_tokens, output_tokens,
     cache_creation_input_tokens, cache_read_input_tokens)
SELECT pg_temp.legacy_uuid(u.message_uuid), u.task_id, u.parent_uuid, u.input_tokens,
       u.output_tokens, u.cache_creation_input_tokens, u.cache_read_input_tokens
  FROM task_message_usage u
    ON CONFLICT DO NOTHING;

UPDATE feedback SET message_uuid = pg_temp.legacy_uuid(message_uuid)::text
 WHERE message_uuid IS NOT NULL;

-- Positions were renumbered, so every live cursor restarts once.
DELETE FROM task_conversation_changes
 WHERE task_id IN (SELECT task_id FROM conversation_frozen);
INSERT INTO task_conversation_position (task_id, position)
SELECT f.task_id,
       COALESCE((SELECT MAX(position) FROM frozen_order o
                  WHERE o.task_id = f.task_id AND o.parent IS NULL), 0)
  FROM conversation_frozen f
    ON CONFLICT (task_id) DO UPDATE SET position = EXCLUDED.position;
-- freeze:end
