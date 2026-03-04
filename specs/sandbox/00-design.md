# Sandbox, Preview & Files

A sandbox is an isolated execution environment provisioned for each task. It hosts the agent, workspace files, and optional preview servers that let users see their running application.

## What Users Can Do

### Sandbox Lifecycle
- **Provision** — happens automatically when a task is created. The sandbox initializes with the repository's configured image.
- **Stop** — pause the sandbox to free resources; workspace files are preserved.
- **Start** — resume a stopped sandbox with files intact.
- **Restart** — recreate the execution environment while keeping workspace files.
- **Cleanup** — permanently remove the sandbox and all its resources.

### Preview
- **View a live preview** — see the running application inside the sandbox through a browser-like panel.
- **Switch server tabs** — when multiple servers run in the sandbox, each gets its own tab.
- **Inspect elements** — use a built-in inspector overlay for layout debugging.
- **Resize the viewport** — test responsive layouts at different widths.
- **View console output** — see application logs and errors from the running servers.

### Files
- **Browse files** — list the workspace directory tree with git status annotations.
- **Read files** — view the contents of any workspace file.
- **Write files** — create or update files directly in the workspace.
- **Search files** — find files by name or glob pattern.
- **View git diff** — see what the agent (or the user) has changed.

> **Note:** FILE-06 (verifying files reflect agent changes) lives in [agent/01-agent-scenarios.md](../agent/01-agent-scenarios.md) because it tests agent execution outcomes rather than standalone file operations.

## How It Works

1. Each task gets its own sandbox with a dedicated workspace volume.
2. The sandbox image comes from the task's repository configuration.
3. Stopping a sandbox preserves the workspace volume — files survive stop/start cycles.
4. Preview servers are defined in the repository configuration and start automatically inside the sandbox.
5. File operations talk directly to the sandbox — no intermediate storage.

## Key Concepts

| Concept | Meaning |
|---------|---------|
| Sandbox | An isolated execution environment provisioned per task |
| Workspace | The file system inside the sandbox where code lives |
| Preview server | A web server running the user's application, accessible through the UI |
| Git status | Tracked/modified/untracked state shown alongside the file tree |
