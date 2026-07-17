-- Persist two extra GitHub PR signals directly on `task_bindings`, alongside
-- the existing `external_status*` columns (see 0064): the PR's review decision
-- (was it approved?) and its mergeable state (does it have conflicts?).
--
-- Dedicated columns rather than keys inside `destination_config` because
-- `upsert()` overwrites `destination_config` wholesale and would clobber them.
-- Written best-effort by the GitHub webhook path: the review decision on
-- `pull_request_review` submissions, the mergeable state on the per-event
-- merge-conflict check (which polls GitHub's async-computed value).

ALTER TABLE task_bindings
    -- Latest review verdict for the PR. Canonical values mirror GitHub:
    -- approved | changes_requested | review_required | dismissed | commented.
    -- NULL means no review has been recorded yet.
    ADD COLUMN IF NOT EXISTS external_review_decision text,
    -- GitHub's coarse mergeability state:
    -- clean | dirty | blocked | behind | unstable | has_hooks | draft.
    -- `dirty` means the PR has merge conflicts. NULL means never computed /
    -- not applicable (e.g. a closed PR). `unknown` is never persisted — an
    -- inconclusive read leaves the previous value untouched.
    ADD COLUMN IF NOT EXISTS external_mergeable_state text;
