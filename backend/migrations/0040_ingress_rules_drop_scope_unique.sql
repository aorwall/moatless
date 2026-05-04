-- Drop the UNIQUE constraint on (provider, scope_matcher) introduced in
-- 0034, so multiple ingress rules can share the same scope.
--
-- Used by Linear: a single team UUID can host several listeners, each with
-- its own (require_labels, trigger_states, terminal_states) filter in
-- event_filter JSONB.
--
-- Recreate the index as non-unique to keep webhook lookups fast.

DROP INDEX IF EXISTS "ingress_rules_provider_scope_idx";

CREATE INDEX IF NOT EXISTS "ingress_rules_provider_scope_idx"
    ON "ingress_rules" ("provider", "scope_matcher");
