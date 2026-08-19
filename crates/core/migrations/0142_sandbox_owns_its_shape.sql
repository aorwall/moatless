-- The Sandbox carries its own access control and pod shape.
--
-- Migration 0135 gave Sandboxes their own primary key and pointed them at a
-- Workspace. This adds the columns the control plane decides on: who may read
-- and control the pod (`scope`, `owner_user_id`), the identity the pod acts as
-- (`run_as_user_id`), what it checks out (`branch`), whether anyone watches it
-- (`headless`), and the wake signal (`pending_work_at`).
--
-- `run_as_user_id` mirrors `Actor::run_as`: the Workspace's account outranks the
-- owner of the work. `scope` + `owner_user_id` are the access-check pair, and
-- they carry the same two values the `tasks` constraint allows.

ALTER TABLE sandboxes
    ADD COLUMN IF NOT EXISTS scope text NOT NULL DEFAULT 'user',
    ADD COLUMN IF NOT EXISTS owner_user_id text,
    ADD COLUMN IF NOT EXISTS run_as_user_id text,
    ADD COLUMN IF NOT EXISTS branch text,
    ADD COLUMN IF NOT EXISTS headless boolean NOT NULL DEFAULT false,
    ADD COLUMN IF NOT EXISTS pending_work_at timestamptz;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'sandboxes_scope_check'
    ) THEN
        ALTER TABLE sandboxes
            ADD CONSTRAINT sandboxes_scope_check
            CHECK (scope IN ('global', 'user'));
    END IF;
END $$;

-- The initial member of each Sandbox: the Task whose id names it, else the
-- oldest member. This is the tie-break `allocate_generation` used to resolve
-- the row's shape from, stated once here so the backfill and the row creation
-- below cannot disagree.
CREATE TEMP TABLE sandbox_initial_member ON COMMIT DROP AS
SELECT DISTINCT ON (COALESCE(sandbox_id, task_id))
       COALESCE(sandbox_id, task_id) AS sid,
       COALESCE(workspace_id, 'ws_' || repository_id) AS workspace_id,
       scope,
       COALESCE(owner_user_id, created_by) AS owner,
       branch,
       headless
  FROM tasks
 ORDER BY COALESCE(sandbox_id, task_id),
          (task_id = COALESCE(sandbox_id, task_id)) DESC,
          created_at_db NULLS LAST,
          task_id;

UPDATE sandboxes s
   SET scope = m.scope,
       owner_user_id = m.owner,
       run_as_user_id = COALESCE(NULLIF(w.run_as_user_id, ''), m.owner),
       branch = m.branch,
       headless = m.headless
  FROM sandbox_initial_member m, workspaces w
 WHERE s.sandbox_id = m.sid
   AND w.id = s.workspace_id;

-- Create the rows the lazy birth used to make on the first operation, so the
-- control plane can read a Sandbox's shape without reading its member Task.
--
-- The join on `workspaces` keeps `sandboxes_workspace_id_fkey` (ON DELETE
-- RESTRICT) satisfiable: a Task naming a workspace that does not resolve gets
-- no row, and the lifecycle then fails by name instead of diverging silently.
--
-- `storage_reclaimed_at` is stamped because a Sandbox that never started owns
-- no volume. `persist_intent` clears it whenever intent moves toward Running,
-- which is what provisions the volume, so a first start reopens the reclaim
-- path. Without the stamp the reaper would offer every never-started Sandbox
-- for reclaim on its first cold pass.
INSERT INTO sandboxes (
    sandbox_id, workspace_id, scope, owner_user_id, run_as_user_id,
    branch, headless, storage_reclaimed_at
)
SELECT m.sid,
       m.workspace_id,
       m.scope,
       m.owner,
       COALESCE(NULLIF(w.run_as_user_id, ''), m.owner),
       m.branch,
       m.headless,
       NOW()
  FROM sandbox_initial_member m
  JOIN workspaces w ON w.id = m.workspace_id
ON CONFLICT (sandbox_id) DO NOTHING;

-- Every member Task names its Sandbox. `allocate_generation` used to bind this
-- on the first operation; the row now exists from creation, so the pointer does
-- too and `reconcile_candidates` sees every member.
UPDATE tasks t
   SET sandbox_id = s.sandbox_id
  FROM sandboxes s
 WHERE t.sandbox_id IS NULL
   AND s.sandbox_id = t.task_id;

-- The fleet-wide wake scan reads only Sandboxes with work waiting.
CREATE INDEX IF NOT EXISTS sandboxes_pending_work_idx
    ON sandboxes (sandbox_id)
    WHERE pending_work_at IS NOT NULL;
