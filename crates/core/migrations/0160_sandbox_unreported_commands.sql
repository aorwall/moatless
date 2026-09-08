-- Which `moat cmd` commands still owe the agent an outcome, as the pod last
-- reported them. The registry is in-memory and its journal rides the S3
-- workspace snapshot, so a replacement whose restore fails loses both. This
-- table is the second copy, and it does not depend on the snapshot.
--
-- Keyed by task rather than by sandbox: a shared sandbox runs one
-- `CommandRegistry` per Member and each mirrors its own set absolutely, so a
-- sandbox-keyed row would let two Members overwrite each other.
--
-- `revision` is the writing registry's own counter, taken under its entries
-- lock and compared here because the write happens off that lock and two can
-- arrive out of order. A take clears `commands` and leaves `revision`, so the
-- pod seeds its counter from the clock to outrun what a prior pod left.
CREATE TABLE IF NOT EXISTS sandbox_unreported_commands (
    task_id    text PRIMARY KEY REFERENCES tasks(task_id) ON DELETE CASCADE,
    commands   jsonb NOT NULL DEFAULT '[]'::jsonb,
    revision   bigint NOT NULL DEFAULT 0,
    updated_at timestamptz NOT NULL DEFAULT NOW()
);
