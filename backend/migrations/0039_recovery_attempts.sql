-- Per-message counter for sandbox-driven agent-run recovery attempts.
-- Each call to the RecoverInterruptedRun RPC increments this on the
-- interrupted user-turn row; once it reaches MAX_RECOVERY_ATTEMPTS the
-- DAO refuses further recoveries and marks the turn aborted.
ALTER TABLE task_messages
    ADD COLUMN IF NOT EXISTS recovery_attempts INT NOT NULL DEFAULT 0;
