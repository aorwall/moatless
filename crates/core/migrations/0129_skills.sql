-- Skill activation as a field rather than a `/skill` prefix in the message text.
--
-- Three levels, all plain name lists. The SKILL.md body never enters the
-- database or the wire: the sandbox resolves each name against the repo's skill
-- roots and the agent's installed home skills dir.
--   * task_messages.skills — what a single send activates.
--   * tasks.skills         — what every fresh agent session on this task
--                            re-activates, so a restart does not lose them.
--   * loop_configs.skills  — what a Loop stamps onto the tasks it spawns.
--
-- NOT NULL DEFAULT '{}' mirrors loop_configs.tag_ids: an absent list is an empty
-- list, so every read binds a plain Vec<String>.

ALTER TABLE "task_messages"
    ADD COLUMN IF NOT EXISTS "skills" text[] NOT NULL DEFAULT '{}'::text[];

ALTER TABLE "tasks"
    ADD COLUMN IF NOT EXISTS "skills" text[] NOT NULL DEFAULT '{}'::text[];

ALTER TABLE "loop_configs"
    ADD COLUMN IF NOT EXISTS "skills" text[] NOT NULL DEFAULT '{}'::text[];
