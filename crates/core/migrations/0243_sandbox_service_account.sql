-- The Kubernetes ServiceAccount the Sandbox's pod was last built with, written
-- before the pod is (re)built. NULL for the deployment's default account, and
-- for a pod built before this column existed until its next rebuild.
ALTER TABLE sandboxes ADD COLUMN IF NOT EXISTS service_account text;
