-- A row is born when lifecycle allocates an operation generation, before the
-- atomic intent write. If the process exits in that interval, the default-born
-- row must retain the same semantics as row absence: converged Stopped with no
-- intent timestamp, so every real pending Message can wake it.
--
-- Rows already born with the previous NOW() default cannot be distinguished
-- from rows whose Stopped intent was explicitly persisted, so this migration
-- changes only the default for future rows.
ALTER TABLE sandboxes
    ALTER COLUMN desired_state_updated_at SET DEFAULT 'epoch'::timestamptz;
