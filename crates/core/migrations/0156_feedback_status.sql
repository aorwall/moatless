-- Triage state of a feedback note: what a reader decided about it, as opposed
-- to `category`, which records what the note is about. Every row starts at
-- `open`; a reader working the inbox moves it, and `status_updated_by` names
-- who did.
--
-- Values are `FeedbackStatus` (crates/types/src/feedback.rs). Left as text
-- rather than an enum type so adding a status stays a code change.

ALTER TABLE feedback ADD COLUMN IF NOT EXISTS status text NOT NULL DEFAULT 'open';
ALTER TABLE feedback ADD COLUMN IF NOT EXISTS status_note text;
ALTER TABLE feedback ADD COLUMN IF NOT EXISTS status_updated_by text;
ALTER TABLE feedback ADD COLUMN IF NOT EXISTS status_updated_at timestamptz;

CREATE INDEX IF NOT EXISTS feedback_status_idx ON feedback (status);
