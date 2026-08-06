-- Replace resume_session_id (text, never used) with resume (boolean, NOT NULL, default true)
ALTER TABLE task_messages DROP COLUMN IF EXISTS resume_session_id;
ALTER TABLE task_messages ADD COLUMN resume boolean NOT NULL DEFAULT true;
