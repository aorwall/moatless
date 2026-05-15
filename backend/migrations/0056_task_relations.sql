-- Typed relations between tasks. Replaces the single tasks.parent_task_id
-- column with an extensible (from_task, to_task, relation_type) model.
-- Initial relation_type is 'created_by'; future types (e.g. 'related_to',
-- 'blocks') need no schema change.

CREATE TABLE IF NOT EXISTS task_relations (
    id            TEXT PRIMARY KEY,
    from_task_id  TEXT NOT NULL REFERENCES tasks(task_id) ON DELETE CASCADE,
    to_task_id    TEXT NOT NULL REFERENCES tasks(task_id) ON DELETE CASCADE,
    relation_type TEXT NOT NULL,
    context       JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (from_task_id, to_task_id, relation_type)
);

CREATE INDEX IF NOT EXISTS idx_task_relations_from
    ON task_relations (from_task_id, relation_type);
CREATE INDEX IF NOT EXISTS idx_task_relations_to
    ON task_relations (to_task_id, relation_type);

-- Backfill existing parent links as `created_by` relations.
INSERT INTO task_relations (id, from_task_id, to_task_id, relation_type, context)
SELECT 'tr_' || gen_random_uuid()::text,
       task_id,
       parent_task_id,
       'created_by',
       '{"backfilled": true}'::jsonb
FROM tasks
WHERE parent_task_id IS NOT NULL
ON CONFLICT DO NOTHING;

-- Drop the old denormalised column. Relations table is now the source of truth.
DROP INDEX IF EXISTS tasks_parent_task_id_idx;
ALTER TABLE tasks DROP COLUMN IF EXISTS parent_task_id;
