-- Skills become records.
--
-- A skill used to exist only for as long as a cached HTTP response: read out
-- of git whenever somebody asked for the list, held for a few minutes, and
-- forgotten. There was nothing durable to attach a setting to, so no setting
-- could be offered. Store them.
--
-- `synced_version` is whatever the source calls its current state (for GitHub,
-- the default branch's head commit SHA). A sync compares it against the source
-- before spending a fetch, so the unchanged case costs one cheap call.

ALTER TABLE "plugins" ADD COLUMN IF NOT EXISTS "synced_version" text;
ALTER TABLE "plugins" ADD COLUMN IF NOT EXISTS "synced_at" timestamptz;

CREATE TABLE IF NOT EXISTS "plugin_skills" (
    "plugin_id"   text NOT NULL REFERENCES "plugins"("id") ON DELETE CASCADE,
    "name"        text NOT NULL,
    "description" text NOT NULL DEFAULT '',
    PRIMARY KEY ("plugin_id", "name")
);
