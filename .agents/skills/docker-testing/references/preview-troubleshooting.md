# Preview containers

Use [preview-servers](../../preview-servers/SKILL.md) for the observe, override,
and promote workflow. Workspace `servers[]` entries create separate Docker server
containers through [container_builder.rs](../../../../crates/sandbox-docker/src/container_builder.rs).

Read `moat tasks status <task-id> --json`, then bounded logs for the returned
server name. Inspect the actual container command, working directory, mounts,
exit code, restarts, and published port. A listening TCP socket does not prove
that the application returns the expected page.

A live override can test a corrected command or environment. Promote the working
configuration to the workspace owner and recreate/redeploy as required by that
change. Do not edit repository API fields for server or image configuration.
