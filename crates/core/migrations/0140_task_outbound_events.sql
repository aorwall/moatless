-- The external events a Task caused: a comment it posted, a review it
-- submitted, a commit it pushed.
--
-- A Task is connected to external things in two ways, and this is the second
-- one. `task_bindings` records the *subject* it is bound to — an issue, a pull
-- request, a channel — and drives delivery to it. This table records the
-- individual events it caused under those subjects. Both last as long as the
-- Task.
--
-- # Direction
--
-- Every row is outbound: the Task made this happen. The inbound side lives on
-- `task_messages.external_event_id`, which records the external event a
-- Message arrived as. Keeping the two apart is what lets a reader tell "I
-- already received this" (a redelivered webhook) from "I caused this" (my own
-- work echoing back), which are different questions with different answers.
--
-- A row is written under the id the *inbound* event will carry, so matching an
-- arriving webhook against it is equality on one column. Where the provider
-- does not hand the writer that id, the Adapter maps between the two in one
-- place — for GitHub, a push records `github:commit:<sha>` and the
-- `pull_request.synchronize` that follows aliases to it, because the pusher
-- cannot know the pull request number the notification will be keyed on.
--
-- Keyed on the id alone, because that is all the writer has: a `moat` command
-- in a sandbox holds `TASK_ID` and the user token, and nothing that names the
-- tool call it runs inside.
CREATE TABLE IF NOT EXISTS "task_outbound_events" (
    "external_event_id" text PRIMARY KEY,
    "task_id"           text NOT NULL,
    "provider"          text NOT NULL,
    -- The provider's own name for where the event happened, `owner/repo` for
    -- GitHub. Same meaning as `task_messages.external_source_name`.
    "source_name"       text,
    "created_at"        timestamptz NOT NULL DEFAULT now()
);

-- Removing a Task removes its events: a row naming a Task nothing can deliver
-- to would keep suppressing an event for nobody.
ALTER TABLE "task_outbound_events"
    ADD CONSTRAINT "task_outbound_events_task_fk"
    FOREIGN KEY ("task_id") REFERENCES "tasks" ("task_id") ON DELETE CASCADE;

CREATE INDEX IF NOT EXISTS "task_outbound_events_task_idx"
    ON "task_outbound_events" USING btree ("task_id");
