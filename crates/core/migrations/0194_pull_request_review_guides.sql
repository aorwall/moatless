-- Guided reviews: an agent-written tour of a pull request, stored once per
-- pull request and replaced when a new one is written. `head_sha` is the head
-- the guide was written against; a different head on the host means stale.
CREATE TABLE IF NOT EXISTS pull_request_review_guides (
    host TEXT NOT NULL,
    repository TEXT NOT NULL,
    number BIGINT NOT NULL,
    head_sha TEXT NOT NULL,
    -- `{ "stops": [...] }` in T3's `PullRequestReviewGuideStop` shape, already
    -- held to the change's lines.
    guide JSONB NOT NULL,
    task_id TEXT NOT NULL,
    agent_profile TEXT,
    model TEXT,
    generated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (host, repository, number)
);

-- The guide run in progress for a pull request, or the last one that failed.
-- A run that stores its guide deletes its row.
CREATE TABLE IF NOT EXISTS pull_request_review_guide_runs (
    host TEXT NOT NULL,
    repository TEXT NOT NULL,
    number BIGINT NOT NULL,
    run_id TEXT NOT NULL UNIQUE,
    head_sha TEXT NOT NULL,
    base_sha TEXT NOT NULL,
    -- NULL between the claim and the creation of the Task that writes it.
    task_id TEXT,
    status TEXT NOT NULL CHECK (status IN ('running', 'failed')),
    detail TEXT,
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    failed_at TIMESTAMPTZ,
    PRIMARY KEY (host, repository, number)
);

CREATE INDEX IF NOT EXISTS pull_request_review_guide_runs_task_idx
    ON pull_request_review_guide_runs (task_id);
