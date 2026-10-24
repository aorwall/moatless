-- A Task on a Workspace that places no repository has no repository to name.
-- Existing rows keep theirs; only a Task on a blank Workspace is written NULL.

ALTER TABLE tasks ALTER COLUMN repository_id DROP NOT NULL;
