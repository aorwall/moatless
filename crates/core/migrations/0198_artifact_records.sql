-- Artifact records: JSON a reader's page saved against an Artifact, kept across
-- its versions. A page may save records only when its latest publish set
-- `accepts_data`; `notify_on_data` also queues a message to the Task's agent
-- for each record a reader saves.
--
-- A record with a `key` is replaced by the next save under that key; one
-- without is appended.

ALTER TABLE task_artifacts ADD COLUMN IF NOT EXISTS accepts_data boolean NOT NULL DEFAULT false;
ALTER TABLE task_artifacts ADD COLUMN IF NOT EXISTS notify_on_data boolean NOT NULL DEFAULT false;

CREATE TABLE IF NOT EXISTS artifact_records (
    record_id uuid PRIMARY KEY,
    artifact_id text NOT NULL REFERENCES task_artifacts(artifact_id) ON DELETE CASCADE,
    key text,
    data jsonb NOT NULL,
    submitted_by text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS artifact_records_key_idx
    ON artifact_records (artifact_id, key) WHERE key IS NOT NULL;
CREATE INDEX IF NOT EXISTS artifact_records_updated_idx
    ON artifact_records (artifact_id, updated_at);
