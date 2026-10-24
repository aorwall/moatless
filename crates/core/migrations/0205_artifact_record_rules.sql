-- Record rules: per key prefix, which record operations a reader's page may
-- perform and which of its writes queue a message to the Task's agent. NULL
-- keeps the rule-free behaviour, where a page may do anything. The latest
-- publish decides.
--
-- `writer` says whether a record came from a page or from a user who can
-- change the Task, such as its agent. Records written before this column are
-- backfilled `page`, so a page never mistakes one of them for the agent's.

ALTER TABLE task_artifacts ADD COLUMN IF NOT EXISTS record_rules jsonb;

ALTER TABLE artifact_records ADD COLUMN IF NOT EXISTS writer text NOT NULL DEFAULT 'page';
