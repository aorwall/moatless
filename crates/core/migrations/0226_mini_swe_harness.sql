ALTER TABLE trajectory_sessions DROP CONSTRAINT trajectory_sessions_harness_check;
ALTER TABLE trajectory_sessions ADD CONSTRAINT trajectory_sessions_harness_check CHECK (harness IN ('claude-code', 'codex', 'pi', 'mini-swe'));
ALTER TABLE conversation_messages DROP CONSTRAINT conversation_messages_harness_check;
ALTER TABLE conversation_messages ADD CONSTRAINT conversation_messages_harness_check CHECK (harness IN ('claude-code', 'codex', 'pi', 'mini-swe'));
