-- Which record keys changed on an Artifact, in commit order, so a page
-- subscribed to its records hears of each change on whichever replica it is
-- connected to. Each record write bumps `task_artifacts.record_seq` in its own
-- transaction, and the row lock that takes is what makes `seq` order commit
-- order. The DAO prunes old changes; a subscriber further behind than the
-- log reaches reads every record again.

ALTER TABLE task_artifacts ADD COLUMN IF NOT EXISTS record_seq bigint NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS artifact_record_changes (
    artifact_id text NOT NULL REFERENCES task_artifacts(artifact_id) ON DELETE CASCADE,
    seq bigint NOT NULL,
    key text NOT NULL,
    PRIMARY KEY (artifact_id, seq)
);
