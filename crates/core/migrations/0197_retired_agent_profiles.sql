-- Rewrite Agent profile values `AgentProfile::parse` rejects. One such `tasks`
-- row fails every `GET /api/v1/tasks` page it lands on with a 500.
-- Commit db6832031 renamed `claude-code-tmux` to `claude-code-cli` without
-- rewriting stored rows; their sandboxes already carry `use_tmux = false`.
-- `not-an-agent` is an E2E row written before profile validation.

UPDATE tasks        SET agent_profile = 'claude-code-cli' WHERE agent_profile = 'claude-code-tmux';
UPDATE sandboxes    SET agent_profile = 'claude-code-cli' WHERE agent_profile = 'claude-code-tmux';
UPDATE task_turns   SET agent_profile = 'claude-code-cli' WHERE agent_profile = 'claude-code-tmux';
UPDATE loop_configs SET agent_profile = 'claude-code-cli' WHERE agent_profile = 'claude-code-tmux';

UPDATE tasks        SET agent_profile = 'claude-code' WHERE agent_profile = 'not-an-agent';
UPDATE sandboxes    SET agent_profile = 'claude-code' WHERE agent_profile = 'not-an-agent';
UPDATE task_turns   SET agent_profile = 'claude-code' WHERE agent_profile = 'not-an-agent';
UPDATE loop_configs SET agent_profile = 'claude-code' WHERE agent_profile = 'not-an-agent';
