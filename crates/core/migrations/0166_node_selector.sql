-- Extra node-selector labels for sandbox pods, merged over the deployment-wide
-- `K8S_SANDBOX_NODE_SELECTOR`. A Loop's value outranks its Workspace's, resolved
-- once into `sandboxes.node_selector` at creation. NULL is the old behavior.
-- See docs/runbooks/sandbox-node-pools.md.
ALTER TABLE workspaces ADD COLUMN IF NOT EXISTS node_selector jsonb;
ALTER TABLE loop_configs ADD COLUMN IF NOT EXISTS node_selector jsonb;
ALTER TABLE sandboxes ADD COLUMN IF NOT EXISTS node_selector jsonb;

-- Stop a Loop's sandbox as soon as its agent run finishes rather than holding
-- the pod for the idle TTL. Its own flag because `ttl_minutes = 0` already means
-- "pinned indefinitely, never reap".
ALTER TABLE loop_configs ADD COLUMN IF NOT EXISTS stop_when_done boolean;
ALTER TABLE sandboxes ADD COLUMN IF NOT EXISTS stop_when_done boolean NOT NULL DEFAULT false;
