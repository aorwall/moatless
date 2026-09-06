# Docker network diagnosis

Read this for a connection failure between the backend, sandbox, and preview.

1. Resolve the task's `sandboxId`, then inspect its containers with `docker ps -a`
   and `docker inspect`. Compare `.NetworkSettings.Networks`, published ports,
   and the actual configured backend URL.
2. Test from the container where the failing request originates. `localhost`
   names that container's network namespace; do not assume it names the host or
   backend. Use the configured service name on the shared Docker network.
3. Compare a direct server request with the returned preview URL. Direct success
   plus proxy failure points to port publication, proxy routing, or authentication.
4. Read backend and server logs around the same timestamp. Check mounts and working
   directories if the server cannot find repository files.

Implementation owners are [provider.rs](../../../../crates/sandbox-docker/src/provider.rs)
and [container_builder.rs](../../../../crates/sandbox-docker/src/container_builder.rs).
Inspect their current network and port setup before proposing host-network or
container-name changes. Do not expose private backend ports to repair a DNS typo.
