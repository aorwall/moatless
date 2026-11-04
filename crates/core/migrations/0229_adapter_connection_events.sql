-- The webhook events a connection has delivered: one row per connection,
-- event type and subject, overwritten by the latest. What a provider's webhook
-- covers is derived from here rather than configured, since no stored setting
-- says which events the provider-side hook was registered with.
CREATE TABLE adapter_connection_events (
    connection_id text NOT NULL REFERENCES adapter_connections(id) ON DELETE CASCADE,
    event_type text NOT NULL,
    subject_key text NOT NULL,
    -- The provider's id for what the event reported, such as a pushed sha.
    external_id text NOT NULL,
    received_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (connection_id, event_type, subject_key)
);

CREATE INDEX adapter_connection_events_event_received_idx
    ON adapter_connection_events (event_type, received_at);
