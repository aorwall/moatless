-- The Linear team a Workspace's T3 Issues page lists, by team key (`SP`).
-- NULL leaves the Workspace with no issue tracker.

ALTER TABLE "workspaces"
    ADD COLUMN IF NOT EXISTS "linear_team" text;
