-- Until when a user's harness is known to refuse turns, written from an
-- `agent.error` whose reason a Loop falls back on. A Loop's Task is moved off
-- a harness while its row's `until` is in the future; an expired row is
-- ignored rather than deleted, and the next refusal overwrites it.
CREATE TABLE harness_cooldowns (
    user_id    text        NOT NULL,
    harness    text        NOT NULL,
    until      timestamptz NOT NULL,
    reason     text        NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, harness)
);
