-- An Artifact: a single-file HTML page an agent published from its Task, stored in S3 so
-- it outlives the sandbox. Each publish to the same `(task_id, slug)` makes a
-- new immutable version.
--
-- A publish reserves `reserved_version` before uploading, then appends to
-- `published_versions` and raises `latest_version`. Only a version listed in
-- `published_versions` is served, so a failed upload whose cleanup also failed
-- stays unreadable. `latest_version = 0` means nothing is published yet.

CREATE TABLE IF NOT EXISTS task_artifacts (
    artifact_id text PRIMARY KEY,
    task_id text NOT NULL REFERENCES tasks(task_id) ON DELETE CASCADE,
    slug text NOT NULL,
    title text NOT NULL,
    reserved_version integer NOT NULL DEFAULT 1,
    latest_version integer NOT NULL DEFAULT 0,
    published_versions integer[] NOT NULL DEFAULT '{}',
    created_by text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (task_id, slug)
);
