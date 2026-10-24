-- Who declared a plugin: `manual` through the admin API, or `deployment`
-- through `PLUGIN_REPOSITORIES_JSON`, which the backend reconciles on every
-- start. A live `deployment` row is read-only to the admin API.
--
-- `disabled_at` is set on a `deployment` row the list no longer declares and
-- cleared when it is declared again, so its skills and activations survive.

-- The CHECK guards every reader: `Plugin.source` decodes strictly, so one row
-- with another value would fail every `SELECT * FROM plugins`.

ALTER TABLE "plugins" ADD COLUMN IF NOT EXISTS "source" text NOT NULL DEFAULT 'manual'
    CHECK ("source" IN ('manual', 'deployment'));

ALTER TABLE "plugins" ADD COLUMN IF NOT EXISTS "disabled_at" timestamptz;
