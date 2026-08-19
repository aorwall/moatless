-- The Sandbox carries the harness its pod runs.
--
-- A pod resolves its Agent harness once, at construction, and every Task that
-- joins it runs that harness. That makes the harness pod shape, so it lives on
-- the Sandbox row beside the rest of it.
--
-- `use_tmux` is stored rather than derived, because deriving it means reading
-- the agent catalog, which the tasks band owns. The tasks band resolves it once
-- at creation; the control plane reads a boolean.

ALTER TABLE sandboxes
    ADD COLUMN IF NOT EXISTS agent_type text,
    ADD COLUMN IF NOT EXISTS use_tmux boolean NOT NULL DEFAULT false;

-- Backfill from the initial member, on the tie-break migration 0142 states.
-- `claude-code-tui` is the one catalog entry that runs over tmux; the catalog
-- (`moatless_tasks::agent::catalog`) stays the authority for every row written
-- after this.
UPDATE sandboxes s
   SET agent_type = m.agent_type,
       use_tmux = (m.agent_type = 'claude-code-tui')
  FROM (SELECT DISTINCT ON (COALESCE(sandbox_id, task_id))
               COALESCE(sandbox_id, task_id) AS sid,
               agent_type
          FROM tasks
         ORDER BY COALESCE(sandbox_id, task_id),
                  (task_id = COALESCE(sandbox_id, task_id)) DESC,
                  created_at_db NULLS LAST,
                  task_id) m
 WHERE s.sandbox_id = m.sid;
