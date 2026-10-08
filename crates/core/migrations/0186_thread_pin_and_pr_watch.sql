-- Per-viewer pin and manual sidebar order for T3. The order keys are
-- fractional-index strings the client mints; rows sort lexicographically by
-- them, so a drag rewrites one row.
ALTER TABLE task_follows ADD COLUMN IF NOT EXISTS pinned_at TIMESTAMPTZ;
ALTER TABLE task_follows ADD COLUMN IF NOT EXISTS pin_order_key TEXT;
ALTER TABLE task_follows ADD COLUMN IF NOT EXISTS active_order_key TEXT;

-- Whether inbound events on this subject wake the Task's agent. Off mutes the
-- Binding without releasing it: status refreshes still land on the row.
ALTER TABLE task_bindings ADD COLUMN IF NOT EXISTS watching BOOLEAN NOT NULL DEFAULT TRUE;
