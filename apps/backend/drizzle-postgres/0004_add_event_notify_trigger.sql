-- Function to send NOTIFY on task_events insert
CREATE OR REPLACE FUNCTION notify_task_event()
RETURNS TRIGGER AS $$
BEGIN
  PERFORM pg_notify('task_events', json_build_object(
    'eventId', NEW.event_id,
    'taskId', NEW.task_id,
    'type', NEW.type,
    'subType', NEW.sub_type
  )::text);
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;
--> statement-breakpoint
-- Trigger to call notify function after INSERT
CREATE TRIGGER task_events_notify_trigger
  AFTER INSERT ON task_events
  FOR EACH ROW
  EXECUTE FUNCTION notify_task_event();
