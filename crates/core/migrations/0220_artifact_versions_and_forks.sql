-- When each Artifact version was published, and in which of its Task's turns,
-- so a page can name its versions and a fork can cut at the turn that
-- published one. Versions published before this table have no row.
CREATE TABLE IF NOT EXISTS task_artifact_versions (
    artifact_id text NOT NULL REFERENCES task_artifacts(artifact_id) ON DELETE CASCADE,
    version integer NOT NULL,
    published_at timestamptz NOT NULL DEFAULT now(),
    turn_number integer,
    PRIMARY KEY (artifact_id, version)
);

-- The Artifact version a fork was copied from. Kept when the source is
-- deleted with its Task, so the fork still says it is one.
ALTER TABLE task_artifacts ADD COLUMN IF NOT EXISTS forked_from_artifact_id text;
ALTER TABLE task_artifacts ADD COLUMN IF NOT EXISTS forked_from_version integer;
