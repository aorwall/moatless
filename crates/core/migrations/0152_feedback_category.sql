-- What a feedback note is about, orthogonal to `surface`, which records only
-- where it was submitted from. Nullable: rows written before this column, and
-- rows whose submitter named no category, carry none.
--
-- Values are `FeedbackCategory` (crates/types/src/feedback.rs). Left as text
-- rather than an enum type so adding a category stays a code change.

ALTER TABLE feedback ADD COLUMN IF NOT EXISTS category text;

CREATE INDEX IF NOT EXISTS feedback_category_idx
    ON feedback (category) WHERE category IS NOT NULL;
