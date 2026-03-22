-- Plugin repos: shared plugin git repositories that can be linked to multiple repositories.

CREATE TABLE IF NOT EXISTS plugin_repos (
    id text PRIMARY KEY,
    name text NOT NULL UNIQUE,
    git_url text NOT NULL,
    created_at timestamptz DEFAULT NOW(),
    updated_at timestamptz DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS repository_plugin_repos (
    repository_id text NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    plugin_repo_id text NOT NULL REFERENCES plugin_repos(id) ON DELETE CASCADE,
    PRIMARY KEY (repository_id, plugin_repo_id)
);

CREATE INDEX IF NOT EXISTS idx_repo_plugin_repos_repo
    ON repository_plugin_repos(repository_id);
