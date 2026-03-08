---
name: moat-cli
description: "This skill should be used when the user asks to 'list tasks', 'create a task', 'list repos', 'create a repo', 'start sandbox', 'stop sandbox', 'send message', 'check status', 'moat login', or mentions the moat CLI, moatless CLI, or sandbox management from the terminal."
---

# Moat CLI

The `moat` CLI is a Rust command-line tool for interacting with the Moatless Vibe platform. It manages tasks, repositories, sandboxes, messages, and channels.

## Installation

```bash
# Build and install to ~/.cargo/bin/moat
cargo install --path crates/cli

# If ~/.cargo/bin is not in PATH, symlink to a directory that is
ln -sf ~/.cargo/bin/moat ~/.local/bin/moat
```

After modifying CLI source code (`crates/cli/src/`), rebuild with `cargo install --path crates/cli`.

## Authentication

### Standalone Mode (default — human users)

```bash
moat login          # Opens browser for OAuth, saves API key
moat whoami         # Verify identity
moat logout         # Remove saved credentials
```

Credentials stored at `~/.config/moatless/credentials.json` with per-host profiles. API key format: `mvk_...`. Sent as `Authorization: Bearer mvk_...`.

### Sandbox Mode (inside K8s pods)

Activated automatically when `USER_AUTH_TOKEN` env var is set. Exchanges token for a session JWT saved to `~/.moat_session`. No manual login needed.

## Global Flags

| Flag | Effect |
|------|--------|
| `--json` | Output raw JSON instead of formatted tables |
| `--verbose` | Print HTTP request details for debugging |

## Command Quick Reference

| Command | Description | Key Flags |
|---------|-------------|-----------|
| `moat login` | Authenticate via browser OAuth | — |
| `moat logout` | Remove saved credentials | — |
| `moat whoami` | Show current user info | — |
| `moat status` | Show API URL, auth mode, profile | — |
| `moat repos list` | List all repositories | — |
| `moat repos get <id>` | Get repository details | — |
| `moat repos create` | Create a repository | `--name`, `--git-url`, `--branch`, `--provider` |
| `moat repos update <id>` | Update a repository | `--name`, `--branch`, `--git-url` |
| `moat tasks list` | List tasks | `--status`, `--limit` |
| `moat tasks get <id>` | Get task details | — |
| `moat tasks create` | Create a task | `--repo`, `--name`, `--message`, `--parent`, `--no-parent`, `--branch` |
| `moat tasks children [id]` | List child tasks | defaults to `$TASK_ID` |
| `moat tasks start <id>` | Start a task | — |
| `moat tasks stop <id>` | Stop a task | — |
| `moat tasks close <id>` | Close a task | — |
| `moat sandbox status <id>` | Show sandbox & agent status | — |
| `moat sandbox start <id>` | Start sandbox for task | — |
| `moat sandbox stop <id>` | Stop sandbox for task | — |
| `moat sandbox restart <id>` | Restart sandbox for task | — |
| `moat messages list <id>` | List messages for a task | — |
| `moat messages send <id>` | Send message to a task | `--text` |
| `moat channels list` | List channel configs | — |
| `moat channels create` | Create channel config | `--provider`, `--source`, `--repo`, `--mode` |
| `moat teams list` | List teams | — |
| `moat teams get <id>` | Get team details | — |
| `moat teams settings get <id>` | Show team settings | — |
| `moat teams settings set <id>` | Update team settings | `--default-repo` |
| `moat settings get` | Show current user settings | — |
| `moat settings set` | Update user settings | `--default-repo` |
| `moat commands` | List available commands | — |
| `moat tags` | List tags | — |

For full flag details and examples, see [Command Reference](references/command-reference.md).

## Common Workflows

### Create a new project

```bash
# 1. Create repository
moat repos create --name my-app --git-url https://github.com/org/my-app

# 2. Create task
moat tasks create --repo <repo-id> --name "Build landing page" --message "Create a responsive landing page"

# 3. Start sandbox
moat sandbox start <task-id>

# 4. Check sandbox status
moat sandbox status <task-id>
```

### Monitor and interact with a running task

```bash
# Check what's running
moat tasks list --status running

# Send follow-up instructions
moat messages send <task-id> --text "Also add a dark mode toggle"

# View conversation history
moat messages list <task-id>
```

### Manage sandbox lifecycle

```bash
moat sandbox status <task-id>     # Check current state
moat sandbox stop <task-id>       # Pause (scale to 0, preserves state)
moat sandbox start <task-id>      # Resume (or auto-provision if needed)
moat sandbox restart <task-id>    # Recreate pod (keeps PVC)
```

### Configure settings

```bash
# View/update user settings
moat settings get
moat settings set --default-repo <repo-id>

# View/update team settings
moat teams list                                          # find team ID
moat teams settings get <team-id>
moat teams settings set <team-id> --default-repo <repo-id>
```

### Scripting with JSON output

```bash
# Get task ID programmatically
moat tasks list --status running --json | jq '.[0].id'

# Get sandbox environment status
moat sandbox status <task-id> --json | jq '.environmentStatus'
```

## Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `MOATLESS_API_URL` | `https://moatless.dev` | API endpoint |
| `BACKEND_API_URL` | (fallback for above) | Alternative env var name |
| `USER_AUTH_TOKEN` | — | Activates sandbox auth mode |
| `TASK_ID` | — | Auto-sets parent task for `tasks create` |
| `MOAT_COOKIE_NAME` | `moatless_session` | Session cookie name |

## Error Handling

All API errors return JSON: `{"error": "message"}`. The CLI prints the error message to stderr. Common errors:

- **401 Unauthorized** — Run `moat login` to re-authenticate
- **404 Not Found** — Check the resource ID
- **500 Internal server error** — Server-side issue; use `--verbose` to inspect the request

## Source Code

CLI source lives at `crates/cli/src/`:

| File | Purpose |
|------|---------|
| `main.rs` | Clap CLI definition, command dispatch |
| `auth.rs` | OAuth login flow, token exchange |
| `credentials.rs` | Credential file management (`~/.config/moatless/`) |
| `config.rs` | Config resolution (env vars → defaults) |
| `client.rs` | HTTP client with auth header injection |
| `output.rs` | Table and JSON formatting |
| `commands/tasks.rs` | Task subcommands |
| `commands/repos.rs` | Repository subcommands |
| `commands/sandbox.rs` | Sandbox subcommands |
| `commands/messages.rs` | Message subcommands |
| `commands/channels.rs` | Channel subcommands |
| `commands/settings.rs` | User settings subcommands |
| `commands/teams.rs` | Team subcommands |
| `commands/info.rs` | `whoami`, `commands`, `tags` |
| `commands/misc.rs` | `status` |

## Development

```bash
# Build and lint
cargo clippy -p moatless-cli -- -D warnings

# Run without installing
cargo run -p moatless-cli -- repos list

# Install after changes
cargo install --path crates/cli
```
