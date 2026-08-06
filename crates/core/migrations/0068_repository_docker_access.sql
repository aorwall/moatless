-- Opt-in Docker-in-Docker access for a repository's sandboxes.
--
-- When true, sandbox pods for this repo get a privileged `dind` sidecar so
-- Testcontainers-based integration tests (Postgres, Redis, …) can run inside the
-- sandbox. Defaults to NULL/false so existing repos stay hardened (no privileged
-- container). See backend/src/sandbox/k8s/pod_builder.rs (build_dind_container).
ALTER TABLE repositories ADD COLUMN docker_access boolean;
