-- The repository link is gone: a plugin now reaches a task through its owner's
-- activation records, not through the repository the task runs against.
--
-- A hard cutover, as decided. Existing links are not converted into global
-- activation records: a link said "this repository uses this plugin", which is
-- a narrower claim than "everybody gets this plugin everywhere", and silently
-- widening it would hand plugins to people who never opted into them. An
-- administrator turns the ones that should be global back on from the admin
-- page.
DROP TABLE IF EXISTS "repository_plugins";
