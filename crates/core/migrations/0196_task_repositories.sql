-- The repositories a Task works in: its Workspace's placements when it was
-- created, and every repository it cloned since. `source` says which.
CREATE TABLE IF NOT EXISTS task_repositories (
    task_id TEXT NOT NULL REFERENCES tasks(task_id) ON DELETE CASCADE,
    repository_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    source TEXT NOT NULL CHECK (source IN ('workspace', 'cloned')),
    added_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (task_id, repository_id)
);

CREATE INDEX IF NOT EXISTS task_repositories_repository_idx
    ON task_repositories (repository_id);

-- Existing Tasks take their Workspace's placements as they are now, which is
-- the closest record there is of what they were created with.
INSERT INTO task_repositories (task_id, repository_id, source)
SELECT t.task_id, wr.repository_id, 'workspace'
FROM tasks t
JOIN workspace_repos wr ON wr.workspace_id = t.workspace_id
ON CONFLICT DO NOTHING;

INSERT INTO task_repositories (task_id, repository_id, source)
SELECT t.task_id, t.repository_id, 'workspace'
FROM tasks t
JOIN repositories r ON r.id = t.repository_id
ON CONFLICT DO NOTHING;
