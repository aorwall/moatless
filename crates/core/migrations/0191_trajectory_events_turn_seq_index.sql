-- no-transaction
-- `response_usage`'s backward scan for a usage line's latest agent Message in
-- its own turn; `(session_id, seq)` alone reads every earlier event of the session.

CREATE INDEX CONCURRENTLY IF NOT EXISTS trajectory_events_session_turn_seq_idx
    ON trajectory_events (session_id, turn_number, seq);
