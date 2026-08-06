-- Fix task_events column naming: Drizzle uses "type" while 0013 used "event_type".
-- On Drizzle-created tables, "type" exists with NOT NULL and "event_type" was added
-- as a separate nullable column. Ensure "type" column exists and is populated.

-- If "type" column doesn't exist (fresh install from 0013), rename event_type to type.
DO $$ BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'task_events' AND column_name = 'type'
    ) THEN
        ALTER TABLE task_events RENAME COLUMN event_type TO type;
    END IF;
END $$;
