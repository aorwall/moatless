-- Activation replaces linking.
--
-- One record says where a plugin, or one skill inside it, is turned on:
-- `user_id IS NULL` means everyone, otherwise it means that one person.
-- `skill_name IS NULL` means the whole plugin, otherwise that one skill.
--
-- `enabled` is stored rather than implied by the row's existence because a
-- personal record has to be able to say *off* — that is how somebody escapes a
-- plugin an administrator turned on for everyone.
--
-- `NULLS NOT DISTINCT` is what makes the unique constraint mean what it reads
-- like: without it Postgres treats every NULL as its own value and a plugin
-- could collect any number of global rows.

CREATE TABLE IF NOT EXISTS "plugin_activations" (
    "plugin_id"  text NOT NULL REFERENCES "plugins"("id") ON DELETE CASCADE,
    "skill_name" text,
    "user_id"    uuid REFERENCES "users"("id") ON DELETE CASCADE,
    "enabled"    boolean NOT NULL,
    "created_at" timestamptz NOT NULL DEFAULT now(),
    "updated_at" timestamptz NOT NULL DEFAULT now(),
    UNIQUE NULLS NOT DISTINCT ("plugin_id", "skill_name", "user_id")
);

-- Resolution reads every global record plus one person's own records, so those
-- are the two shapes worth an index.
CREATE INDEX IF NOT EXISTS "idx_plugin_activations_global"
    ON "plugin_activations"("plugin_id") WHERE "user_id" IS NULL;

CREATE INDEX IF NOT EXISTS "idx_plugin_activations_user"
    ON "plugin_activations"("user_id") WHERE "user_id" IS NOT NULL;
