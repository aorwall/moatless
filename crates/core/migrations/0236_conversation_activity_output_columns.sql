-- A settled call's output, out of `payload`: a transcript read selects
-- `payload` for every activity in its window, and reading the output with it
-- decompressed and shipped every result each time a thread re-rendered. Rows
-- settled before this keep theirs in `payload`; readers take the column first.

ALTER TABLE conversation_activities
    ADD COLUMN IF NOT EXISTS result       jsonb,
    ADD COLUMN IF NOT EXISTS typed_result jsonb;
