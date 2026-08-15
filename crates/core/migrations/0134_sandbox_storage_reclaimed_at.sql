-- When the cold transition reclaimed a stopped sandbox's Deployment and volume,
-- leaving the S3 snapshot as the only copy.
--
-- This is what keeps the cold branch O(newly cold tasks) instead of O(stopped
-- tasks): a sandbox stays `Stopped` forever, so without a durable "already done"
-- mark the reaper would re-head S3, re-allocate a generation, and re-ask
-- Kubernetes about every long-stopped task on every cycle. Cleared whenever
-- intent moves back to Running, because a start provisions a fresh volume.
ALTER TABLE sandboxes ADD COLUMN storage_reclaimed_at TIMESTAMPTZ;

-- The cold branch's read: stopped sandboxes that still own storage. Partial, so
-- the index holds only the rows the branch can act on.
CREATE INDEX sandboxes_reclaimable_idx
    ON sandboxes (desired_state_updated_at)
    WHERE desired_state = 'stopped' AND storage_reclaimed_at IS NULL;
