-- Remove the task-templates concept: the reusable task-defaults table and the
-- tasks column that recorded which template created a task.
ALTER TABLE tasks DROP COLUMN IF EXISTS task_template_id;
DROP TABLE IF EXISTS task_templates CASCADE;
