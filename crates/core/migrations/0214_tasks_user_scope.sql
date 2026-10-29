-- Every Task is `user`-scoped. Migration 0007 set `scope = 'global'` on each
-- Task that was public then, and 0142 copied it to the Task's Sandbox. A global
-- Task made private stays readable by every signed-in user and writable by
-- admins only, not even by its owner. `visibility` alone decides who else may
-- reach a Task, so the scope goes back to `user` on both rows.

-- `tasks_owner_scope_chk` requires an owner on a `user` Task and none on a
-- global one, so both change in one statement: the creator becomes the owner.
-- A Task with no creator stays global.
UPDATE tasks SET scope = 'user', owner_user_id = COALESCE(owner_user_id, created_by)
 WHERE scope = 'global' AND COALESCE(owner_user_id, created_by) IS NOT NULL;
UPDATE sandboxes SET scope = 'user' WHERE scope = 'global';
