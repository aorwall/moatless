-- When the agent did not confirm a person's Stop of this turn in time. Set
-- only while the turn is open; the T3 projection offers Force stop for it.
ALTER TABLE task_turns ADD COLUMN IF NOT EXISTS stop_unresponsive_at timestamptz;
