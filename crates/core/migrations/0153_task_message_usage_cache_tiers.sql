-- task_message_usage recorded only (input, output) tokens, silently dropping
-- the cache tiers. Codex's cache-read is its largest input tier, so per-message
-- cost was not derivable from these rows. Add the two cache columns; existing
-- rows default to 0 (their cache split was never captured and cannot be
-- recovered here).
ALTER TABLE task_message_usage
    ADD COLUMN IF NOT EXISTS cache_creation_input_tokens integer NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS cache_read_input_tokens integer NOT NULL DEFAULT 0;
