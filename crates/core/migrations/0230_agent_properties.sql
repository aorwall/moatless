-- Harness settings a Loop gives the Tasks it spawns (`AgentProperties`).
ALTER TABLE loop_configs ADD COLUMN agent_properties JSONB NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE tasks ADD COLUMN agent_properties JSONB NOT NULL DEFAULT '{}'::jsonb;
