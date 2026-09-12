-- Presentation and Task defaults a Workspace carries for its clients.
--
-- `icon` holds the T3 client's `ProjectIconOverride` object verbatim, so the
-- glyph vocabulary stays in the client's schema rather than in a check here.
-- `default_model` is a model catalog id (`opus`) and `default_effort` a
-- reasoning-effort level (`high`); NULL on either keeps the platform default
-- the agent resolves, so an absent value changes nothing.

ALTER TABLE "workspaces"
    ADD COLUMN IF NOT EXISTS "icon" jsonb,
    ADD COLUMN IF NOT EXISTS "default_model" text,
    ADD COLUMN IF NOT EXISTS "default_effort" text;
