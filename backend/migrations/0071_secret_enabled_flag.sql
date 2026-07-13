-- Add a reversible enable/disable toggle to secrets. A disabled secret keeps
-- its value but is skipped during resolution, so the next scope down
-- (user -> team -> global) fills the key instead. Existing rows stay active.

ALTER TABLE "secrets"
    ADD COLUMN IF NOT EXISTS "enabled" boolean NOT NULL DEFAULT true;
