---
name: docker-testing
description: "Operate and debug a running Moatless Docker stack. Use for sandbox containers, Docker networking, and container communication failures."
---

# Docker diagnostics

Start with `moat docker status --json` and `moat tasks diagnose <task-id>`.
Resolve the sandbox using `moat tasks get <task-id> --json`: its `sandboxId`
can differ from `taskId` when tasks share a sandbox.

Use `docker ps -a` and `docker inspect <container>` to identify the actual sandbox
container, networks, mounts, exit state, and published ports. Include stopped
containers when investigating crashes. Read bounded logs:

```bash
moat tasks logs <task-id> --no-follow --tail 500
docker logs --tail 500 <container>
```

A Script runs as a terminal session inside the sandbox container, not a separate
container; its output is in its terminal, not `moat tasks logs`. Docker mode does
not publish a Script's `port`, so a Script with a `port` has no preview URL under
Docker (`DockerSandboxAdapter`'s `SandboxPortPublisher::publish_port` always
returns `None` — see `docs/deferred-changes.md`).

- [Networking](references/networking.md): connection refused, unreachable backend,
  or wrong hostname.
- [E2E suite](../e2e-testing/SKILL.md): reproducible product tests.
- [CLI](../moat-cli/SKILL.md): authenticated task and workspace operations.

Application APIs require authentication. Prefer the logged-in CLI; a successful
public `/health` request does not authenticate subsequent `/api/v1` requests.
