-- The durable record that something other than a client started a terminal
-- session on a Task, and the one counter that orders those records for the
-- whole deployment.
--
-- The PTY lives in the Sandbox, but the list of sessions a viewer sees is
-- served from `TerminalHub`, which fans out in one process only. `moat tasks
-- scripts run` is a load-balanced HTTP POST, so the replica that starts the
-- session is whichever one the balancer picked, while the person waiting for it
-- holds a WebSocket to a replica chosen earlier and independently. With two
-- replicas the Script ran and roughly half the time the terminal never appeared
-- in the drawer. These two tables are the hop that crosses that gap, as
-- `preview_tab_changes` already does for browser tabs.
--
-- Only a session the environment started is recorded. A session a client opened
-- arrives on that client's own socket, so the replica serving it is already
-- current.

-- The deployment's position counter. Exactly one row, and the `CHECK` is what
-- makes that true rather than conventional.
--
-- No mutation's transaction holds this one: the session is already running in
-- the Sandbox and stored on the writing replica by the time the record is
-- written, so there is nothing for a reader to observe early. That keeps this
-- counter out of the feed lock order in `moatless_core::change_feed` — a writer
-- takes it and no other lock.
CREATE TABLE IF NOT EXISTS terminal_session_position (
    id       boolean PRIMARY KEY DEFAULT TRUE,
    position bigint  NOT NULL DEFAULT 0,
    CONSTRAINT terminal_session_position_is_one_row CHECK (id)
);

INSERT INTO terminal_session_position (id, position) VALUES (TRUE, 0)
    ON CONFLICT (id) DO NOTHING;

-- One record per session the environment started.
--
-- `origin` is the writing replica's `TerminalHub` epoch, and every replica skips
-- its own records: it stored the session before writing this one, and applying
-- it twice would raise the same terminal for its subscribers a second time.
--
-- `summary` is the whole T3 terminal summary, not a pointer to read back. The
-- reader applies it without touching the Sandbox, which is what lets the relay
-- worker run with a database handle and nothing else. Its shape is owned by
-- `moatless_t3code::terminal`; nothing here reads inside it.
--
-- `position` is the primary key, so the reader's `WHERE position > $1 ORDER BY
-- position` needs no further index. `recorded_at` is read by nothing and exists
-- for an operator reading the table by hand; retention counts positions.
--
-- Deliberately no foreign key to `tasks`, matching `preview_tab_changes`: a
-- record naming a Task that has gone away resolves to a terminal nobody is
-- shown, and retention reclaims it on the ordinary schedule.
--
-- Retention is `change_feed::terminal::prune_starts`, deleting below
-- `RETAINED_CHANGES` positions from the counter's head. Nothing resumes from a
-- position here — a replica's cursor starts at the head on every boot, since a
-- session started before it booted belongs to a Script it cannot vouch for — so
-- the window bounds table growth and nothing else.
CREATE TABLE IF NOT EXISTS terminal_session_changes (
    position    bigint PRIMARY KEY,
    origin      text NOT NULL,
    task_id     text NOT NULL,
    terminal_id text NOT NULL,
    summary     jsonb NOT NULL,
    recorded_at timestamptz NOT NULL DEFAULT NOW()
);
