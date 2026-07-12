-- Normalize legacy task statuses that predate the open/closed/error enum.
--
-- Rows created before the TaskStatus enum was reduced to open/closed/error can
-- still carry free-text values like 'running' (tasks.status has no CHECK
-- constraint). The orphan-GC pass fails closed on undecodable statuses — safe,
-- but it warns every cycle and can never reclaim those tasks' leftover K8s
-- resources. Idempotent: only touches rows outside the known set.
UPDATE tasks
SET status = 'closed'
WHERE status NOT IN ('open', 'closed', 'error');
