-- Persist the external status of a binding's connected subject (GitHub PR /
-- Linear issue) directly on `task_bindings`.
--
-- These are dedicated columns rather than keys inside `destination_config`
-- because `upsert()` overwrites `destination_config` wholesale (see
-- backend/src/bindings/dao.rs::upsert) and would clobber any status written
-- there. Status is written by webhook handlers (GitHub `pull_request`,
-- Linear `Issue.update`) and by the Moka-cached live status service on read.

ALTER TABLE task_bindings
    -- Display string. GitHub: open|merged|closed|draft. Linear: the workflow
    -- state name (user-editable, e.g. "In Review").
    ADD COLUMN IF NOT EXISTS external_status text,
    -- Normalized bucket for color-coding / filtering. GitHub:
    -- open|merged|closed|draft. Linear: state.type
    -- (unstarted|started|completed|canceled|backlog|triage).
    ADD COLUMN IF NOT EXISTS external_state_kind text,
    -- Full detail blob (title, url, state id/name, merged flag, …) for clients
    -- that want more than the two summary strings.
    ADD COLUMN IF NOT EXISTS external_status_raw jsonb,
    -- When the status above was last refreshed. NULL means "never persisted";
    -- the read path uses staleness of this column to decide whether a live
    -- refresh is worthwhile.
    ADD COLUMN IF NOT EXISTS external_status_updated_at timestamptz;
