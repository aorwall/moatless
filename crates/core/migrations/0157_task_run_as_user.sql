-- The identity a Task's Sandbox acts as outward — git-host credentials and the
-- plugins it is delivered — when the Task names one itself.
--
-- Null is the ordinary case and keeps the existing rule: the Workspace's
-- `run_as_user_id` outranks the Task's owner, per `Actor::run_as`. A value here
-- outranks both, and only an admin naming a bot can set it (`ScopeContext::can_run_as`).

ALTER TABLE tasks ADD COLUMN IF NOT EXISTS run_as_user_id text;
