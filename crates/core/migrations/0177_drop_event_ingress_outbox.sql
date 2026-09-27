-- The outbox held full `agent.message` payloads between admission and the
-- dispatcher. Agent output now reaches the backend as Trajectory lines, so no
-- event is spooled here any more.
DROP TABLE IF EXISTS event_ingress_outbox;
