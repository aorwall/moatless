-- Record collections: what every record belongs to. An Artifact's own records
-- are its `artifact` collection, whose id is the Artifact's and which its
-- delete cascades to. The other kinds are addressed like Memory, by
-- `(scope_kind, scope_id, name)`, so records the Tasks of one Loop or
-- Workspace write outlive each of those Tasks.
--
-- `record_seq` moves here from `task_artifacts`: each record write bumps its
-- collection's, and `collection_record_changes` logs it.

CREATE TABLE IF NOT EXISTS record_collections (
    collection_id text PRIMARY KEY,
    scope_kind text NOT NULL CHECK (
        scope_kind IN ('artifact', 'task', 'loop', 'workspace', 'repository', 'user', 'global')
    ),
    scope_id text NOT NULL,
    name text NOT NULL,
    artifact_id text UNIQUE REFERENCES task_artifacts(artifact_id) ON DELETE CASCADE,
    record_seq bigint NOT NULL DEFAULT 0,
    created_by_task_id text,
    created_by_user_id text,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (scope_kind, scope_id, name),
    CHECK ((scope_kind = 'artifact') = (artifact_id IS NOT NULL))
);

INSERT INTO record_collections
    (collection_id, scope_kind, scope_id, name, artifact_id, record_seq, created_by_user_id, created_at)
SELECT artifact_id, 'artifact', artifact_id, '', artifact_id, record_seq, created_by, created_at
FROM task_artifacts
ON CONFLICT DO NOTHING;

ALTER TABLE artifact_records RENAME TO collection_records;
ALTER TABLE collection_records RENAME COLUMN artifact_id TO collection_id;
ALTER TABLE collection_records DROP CONSTRAINT IF EXISTS artifact_records_artifact_id_fkey;
ALTER TABLE collection_records ADD CONSTRAINT collection_records_collection_id_fkey
    FOREIGN KEY (collection_id) REFERENCES record_collections(collection_id) ON DELETE CASCADE;
ALTER INDEX IF EXISTS artifact_records_pkey RENAME TO collection_records_pkey;
ALTER INDEX IF EXISTS artifact_records_updated_idx RENAME TO collection_records_updated_idx;

ALTER TABLE artifact_record_changes RENAME TO collection_record_changes;
ALTER TABLE collection_record_changes RENAME COLUMN artifact_id TO collection_id;
ALTER TABLE collection_record_changes
    DROP CONSTRAINT IF EXISTS artifact_record_changes_artifact_id_fkey;
ALTER TABLE collection_record_changes ADD CONSTRAINT collection_record_changes_collection_id_fkey
    FOREIGN KEY (collection_id) REFERENCES record_collections(collection_id) ON DELETE CASCADE;
ALTER INDEX IF EXISTS artifact_record_changes_pkey RENAME TO collection_record_changes_pkey;

ALTER TABLE task_artifacts DROP COLUMN IF EXISTS record_seq;

-- The scoped collections the latest publish lets its page read, as
-- `[{"scope": "loop", "name": "review-runs", "collectionId": "…"}]`.
ALTER TABLE task_artifacts ADD COLUMN IF NOT EXISTS collections jsonb NOT NULL DEFAULT '[]';
