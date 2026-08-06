-- A listing change can now name a Workspace as well as a Task.
--
-- 0104 recorded one subject because one change kind was live: a Task's turn
-- moving. The sidebar's other half is the projects conversations are grouped
-- under, and a project being created, renamed or deleted is a change a watching
-- client has to be told about — a renamed project whose title never updates, and
-- a deleted one that stays in the sidebar forever, are both changes the client
-- can see and nothing tells it about.
--
-- A Workspace change cannot be expressed as a change to the Tasks inside it.
-- Creating a Workspace has no Tasks at all, and the row the client would have to
-- re-read to learn the new title is the *project* row, which a Task change does
-- not carry. So the record names a subject, and the subject is one of the two.
--
-- Exactly one of the two columns is set, and the CHECK is what makes that true
-- rather than conventional: a record naming neither would advance the position
-- and tell nobody anything, and a record naming both would be two changes at one
-- position.
--
-- Deliberately still no foreign key, for the same reason 0104 has none: a record
-- has to outlive the row it names, because the row going away is itself the
-- change a resuming client most needs.

ALTER TABLE task_listing_changes ADD COLUMN IF NOT EXISTS workspace_id text;
ALTER TABLE task_listing_changes ALTER COLUMN task_id DROP NOT NULL;

ALTER TABLE task_listing_changes
    DROP CONSTRAINT IF EXISTS task_listing_changes_names_one_subject;
ALTER TABLE task_listing_changes
    ADD CONSTRAINT task_listing_changes_names_one_subject
    CHECK ((task_id IS NULL) <> (workspace_id IS NULL));
