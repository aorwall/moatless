-- Every Task is `user`-scoped. Migration 0007 set `scope = 'global'` on each
-- Task that was public then, and 0142 copied it to the Task's Sandbox. A global
-- Task made private stays readable by every signed-in user and writable by
-- admins only, not even by its owner. `visibility` alone decides who else may
-- reach a Task, so the scope goes back to `user` on both rows.

UPDATE tasks SET scope = 'user' WHERE scope = 'global';
UPDATE sandboxes SET scope = 'user' WHERE scope = 'global';
