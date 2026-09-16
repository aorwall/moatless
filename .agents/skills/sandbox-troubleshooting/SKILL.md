---
name: sandbox-troubleshooting
description: "Diagnose a stuck, unresponsive, restarted, evicted, OOMKilled, or unavailable Moatless sandbox or agent. Use CLI history, runtime diagnostics, Kubernetes events, and metrics to identify the cause before repair."
---

# Sandbox troubleshooting

Start with the task's current state and history:

```bash
moat tasks diagnose <task-id>
moat tasks status <task-id> --json
moat tasks get <task-id> --json
moat events list <task-id> --errors
moat messages list <task-id> --full
moat tasks logs <task-id> --server sandbox --no-follow --tail 500
```

`diagnose` describes current health; events and messages explain what happened
before a restart or stall. Read `sandboxId` and `workspaceId` from task details.
Several tasks can share a sandbox, so a task ID is not a reliable pod identity.

For Kubernetes, discover the namespace and use an explicit context. Select pods
by `moatless.io/sandbox-id=<sandbox-id>`; include Pending and terminated pods.
Read [kubectl](references/kubectl.md) for permissions, events, and container selection.
For Docker use [docker-testing](../docker-testing/SKILL.md).

Choose the next read from the symptom:

- [Decision tree](references/decision-tree.md): narrow a failure before changing state.
- [Failure modes](references/failure-modes.md): resource and storage implications.
- [Node disruption](references/node-disruption.md): the pod outlived by nothing —
  spot reclaim, consolidation, drift.
- [Commands](references/commands.md): bounded logs and history.
- [Metrics](references/metrics.md): fleet health and adapter delivery.
- [Code map](references/code-map.md): source owners for ambiguous classifications.

Use [preview-servers](../preview-servers/SKILL.md) for a failed server and
[E2E testing](../e2e-testing/SKILL.md) for reproducible smoke verification.
Capture evidence before you destroy a failed pod, and check the other members
first. Restarting the sandbox running your own investigation ends the current
turn: `moat tasks restart $TASK_ID --yes` replaces the pod and wakes you with a
`<sandbox-restart-note>` once it is back, and everything not yet captured is
gone. The `moat-cli` skill states that contract.
