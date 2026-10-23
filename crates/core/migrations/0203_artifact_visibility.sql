-- Who may read an Artifact's pages and records. `public` lets any signed-in
-- user read them whatever the Task's visibility; `private` limits them to
-- whoever can read the Task. The latest publish decides.
--
-- Artifacts published before this column were readable only through their
-- Task, so they are backfilled `private` and only new rows default `public`.

ALTER TABLE task_artifacts ADD COLUMN IF NOT EXISTS visibility text NOT NULL DEFAULT 'private';
ALTER TABLE task_artifacts ALTER COLUMN visibility SET DEFAULT 'public';
