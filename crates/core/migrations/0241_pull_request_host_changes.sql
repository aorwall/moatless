-- Moves each time a bound pull request's host state changes, so every replica
-- can tell a T3 client to re-read the pull requests it shows. A sequence rather
-- than a row: `nextval` takes no row lock, so concurrent webhooks never queue
-- on it, and a gap left by a rolled-back write costs one spare refresh.
CREATE SEQUENCE IF NOT EXISTS pull_request_host_changes;
