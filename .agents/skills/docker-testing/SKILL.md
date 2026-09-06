---
name: docker-testing
description: "Operate and debug a running Moatless Docker stack. Use for sandbox containers, preview container logs, Docker networking, and container communication failures."
---

# Docker diagnostics

Start with `moat docker status --json` and `moat tasks diagnose <task-id>`.
Resolve the sandbox using `moat tasks get <task-id> --json`: its `sandboxId`
can differ from `taskId` when tasks share a sandbox.

Use `docker ps -a` and `docker inspect <container>` to identify the actual sandbox
and server containers, networks, mounts, exit state, and published ports. Include
stopped containers when investigating crashes. Read bounded logs:

```bash
moat tasks logs <task-id> --server sandbox --no-follow --tail 500
docker logs --tail 500 <container>
```

The agent sandbox and each preview server run in separate containers. Do not
look for a preview process in the agent's process namespace. Use the task status
response's server names and URLs; do not infer host ports from task IDs.

- [Networking](references/networking.md): connection refused, unreachable backend,
  wrong hostname, or missing published port.
- [Preview troubleshooting](references/preview-troubleshooting.md): server startup
  and configuration.
- [E2E suite](../e2e-testing/SKILL.md): reproducible product tests.
- [CLI](../moat-cli/SKILL.md): authenticated task and workspace operations.

Application APIs require authentication. Prefer the logged-in CLI; a successful
public `/health` request does not authenticate subsequent `/api/v1` requests.
