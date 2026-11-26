-- `fatal` was an older spelling of the sandbox `error` status in
-- `task_events.sub_type`. The payload's `lifecycle_event: "fatal"` is current
-- and stays.
UPDATE task_events SET sub_type = 'error'
 WHERE type = 'sandbox' AND sub_type = 'fatal';
