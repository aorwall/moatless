-- Memory: what agents learn during a run, kept outside the sandbox and shared
-- by every Task that can reach the entry's scope. An entry is addressed by
-- `(scope_kind, scope_id, key)`; `scope_id` is '' for the `global` scope.
--
-- `expires_at` is recomputed from `ttl_seconds` on every put and verify, so an
-- entry nobody confirms disappears; a NULL `ttl_seconds` never expires. A read
-- treats an expired row as absent, and a put over it starts again at version 1.
-- A claim is held while `claim_expires_at` is in the future and the claiming
-- Task is still open.

CREATE TABLE IF NOT EXISTS memory_entries (
    entry_id text PRIMARY KEY,
    scope_kind text NOT NULL
        CHECK (scope_kind IN ('task', 'loop', 'workspace', 'repository', 'user', 'global')),
    scope_id text NOT NULL,
    key text NOT NULL,
    summary text NOT NULL
        CHECK (char_length(summary) BETWEEN 1 AND 160 AND position(E'\n' IN summary) = 0),
    body text NOT NULL DEFAULT '',
    data jsonb,
    tags text[] NOT NULL DEFAULT '{}',
    citations text[] NOT NULL DEFAULT '{}',
    version integer NOT NULL DEFAULT 1,
    ttl_seconds bigint,
    expires_at timestamptz,
    stale_after_seconds bigint,
    verified_at timestamptz,
    claimed_by_task_id text,
    claim_expires_at timestamptz,
    updated_by_task_id text,
    updated_by_loop_id text,
    updated_by_user_id text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    -- `array_to_tsvector`, not `array_to_string`: a generated column needs an
    -- immutable expression, and Postgres marks `array_to_string` stable.
    search tsvector GENERATED ALWAYS AS (
        to_tsvector('simple', key || ' ' || summary || ' ' || body)
            || array_to_tsvector(tags)
    ) STORED,
    UNIQUE (scope_kind, scope_id, key)
);

CREATE INDEX IF NOT EXISTS memory_entries_search_idx
    ON memory_entries USING GIN (search);
CREATE INDEX IF NOT EXISTS memory_entries_key_prefix_idx
    ON memory_entries (scope_kind, scope_id, key text_pattern_ops);

-- Every write to an entry, newest last; `append` writes only here.
CREATE TABLE IF NOT EXISTS memory_events (
    event_id bigserial PRIMARY KEY,
    entry_id text NOT NULL REFERENCES memory_entries(entry_id) ON DELETE CASCADE,
    op text NOT NULL CHECK (op IN ('put', 'append', 'verify', 'claim', 'release')),
    version integer NOT NULL,
    text text,
    task_id text,
    user_id text,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS memory_events_entry_idx
    ON memory_events (entry_id, event_id DESC);

-- Which version of an entry a Task read in full, so a run that went wrong can
-- be traced to the entry that misled it.
CREATE TABLE IF NOT EXISTS memory_reads (
    entry_id text NOT NULL REFERENCES memory_entries(entry_id) ON DELETE CASCADE,
    task_id text NOT NULL,
    version integer NOT NULL,
    read_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (entry_id, task_id, version)
);

-- The keys, or `prefix*` patterns, whose summaries a Task's system prompt
-- lists. A Loop stamps its LOOP.md `memory.preload` onto the Tasks it opens.
ALTER TABLE tasks
    ADD COLUMN IF NOT EXISTS memory_preload text[] NOT NULL DEFAULT '{}'::text[];
ALTER TABLE loop_configs
    ADD COLUMN IF NOT EXISTS memory_preload text[] NOT NULL DEFAULT '{}'::text[];
