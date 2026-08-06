-- Task templates: named, reusable bundles of task defaults.
CREATE TABLE task_templates (
    id              text PRIMARY KEY NOT NULL,
    name            text NOT NULL,
    description     text,
    icon            text,
    color           text,
    repository_id   text NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    agent_type      text NOT NULL,
    agent_mode      text,
    default_model   text,
    initial_prompt  text,
    skills          jsonb,
    scope           text NOT NULL DEFAULT 'user',
    team_id         text,
    featured        boolean NOT NULL DEFAULT false,
    display_order   integer NOT NULL DEFAULT 0,
    created_by      text,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX task_templates_scope_idx       ON task_templates (scope);
CREATE INDEX task_templates_created_by_idx  ON task_templates (created_by);
CREATE INDEX task_templates_repository_id   ON task_templates (repository_id);
CREATE INDEX task_templates_featured_idx    ON task_templates (featured, display_order) WHERE featured = true;

-- Track which template was used to create a task (for recent-used ordering).
ALTER TABLE tasks ADD COLUMN task_template_id text REFERENCES task_templates(id) ON DELETE SET NULL;
CREATE INDEX tasks_template_id_idx    ON tasks (task_template_id);
CREATE INDEX tasks_template_recent_idx ON tasks (task_template_id, created_by, created_at DESC) WHERE task_template_id IS NOT NULL;
