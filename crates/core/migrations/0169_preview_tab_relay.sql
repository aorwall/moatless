-- The durable record that something other than a client opened a browser tab on
-- a Task, and the one counter that orders those records for the whole
-- deployment.
--
-- A preview tab itself stays in process, deliberately: it is UI state whose
-- value dies with the servers it points at. What could not stay in process is
-- the *announcement*. `moat tasks scripts run` is a load-balanced HTTP POST, so
-- the replica that opens the tab is whichever one the balancer picked, while
-- the person waiting for it holds a WebSocket to a replica chosen earlier and
-- independently. The in-process fan-out cannot cross that gap, so with two
-- replicas the tab opened and roughly half the time nobody saw it. These two
-- tables are the hop that crosses it.
--
-- Only an environment-initiated open is recorded, and that is the whole rule:
-- every other preview mutation arrives on the issuing client's own socket, so
-- that client is already current and the other replicas hold no tab worth
-- correcting.

-- The deployment's position counter. Exactly one row, and the `CHECK` is what
-- makes that true rather than conventional.
--
-- Unlike `task_listing_position`, no mutation's transaction holds this one: the
-- tab is already stored and published on the writing replica by the time the
-- record is written, so there is nothing for a reader to observe early. That
-- also keeps this counter out of the feed lock order in
-- `moatless_core::change_feed` — a writer takes it and no other lock.
CREATE TABLE IF NOT EXISTS preview_tab_position (
    id       boolean PRIMARY KEY DEFAULT TRUE,
    position bigint  NOT NULL DEFAULT 0,
    CONSTRAINT preview_tab_position_is_one_row CHECK (id)
);

INSERT INTO preview_tab_position (id, position) VALUES (TRUE, 0)
    ON CONFLICT (id) DO NOTHING;

-- One record per tab the environment opened.
--
-- `origin` is the writing replica's `PreviewTabs` epoch, and every replica skips
-- its own records: it applied the open before writing this one, and applying it
-- twice would announce the same tab to its subscribers a second time.
--
-- The record carries the whole open (`tab_id`, `url`) rather than a pointer to
-- read back, because there is nothing durable to read back — the tab exists only
-- in the origin's memory.
--
-- `position` is the primary key, so the reader's `WHERE position > $1 ORDER BY
-- position` needs no further index. `recorded_at` is read by nothing and exists
-- for an operator reading the table by hand; retention counts positions.
--
-- Deliberately no foreign key to `tasks`. A reader authorizes each record
-- against the viewer asking for it, so a record naming a Task that has gone away
-- resolves to a tab nobody is shown, and retention reclaims it on the ordinary
-- schedule.
--
-- Retention is `change_feed::preview::prune_opens`, deleting below
-- `RETAINED_CHANGES` positions from the counter's head. Nothing resumes from a
-- position here — a replica's cursor starts at the head on every boot, since a
-- tab opened before it started belongs to a Script it cannot vouch for — so the
-- window bounds table growth and nothing else.
CREATE TABLE IF NOT EXISTS preview_tab_changes (
    position    bigint PRIMARY KEY,
    origin      text NOT NULL,
    task_id     text NOT NULL,
    tab_id      text NOT NULL,
    url         text NOT NULL,
    recorded_at timestamptz NOT NULL DEFAULT NOW()
);
