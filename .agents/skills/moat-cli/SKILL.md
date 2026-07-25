---
name: moat-cli
description: "This skill should be used when the user asks to 'list tasks', 'create a task', 'list repos', 'create a repo', 'start task', 'stop task', 'check status', 'send message', 'moat login', or mentions the moat CLI, moatless CLI, or task management from the terminal."
---

# Moat CLI

The `moat` CLI is a Rust command-line tool for interacting with the Moatless Vibe platform. It manages tasks, repositories, messages, and external integrations (adapters, triggers, bindings).

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

## Commands

Full command list with flags and examples: [references/command-reference.md](references/command-reference.md).

Gotchas:
- Send to a task: `moat send --task <id> --text "…"` — there is NO `moat messages send`.
- `moat tasks create` auto-starts the sandbox; `moat tasks start` resumes a stopped one.
- `moat tasks create --workspace <id-or-name>` replaces `--repo` and carries the
  sandbox's run-config (resources, servers, Docker access). For **child tasks,
  default to the sandbox-only workspace** (e.g. `Moatless`) — no dev server, so
  the pod is small and many fit per node instead of one. Use `Moatless Preview`
  only when the app must actually run, `Moatless Fullstack` only when a Docker
  daemon is needed.
- Cron triggers take 7-field cron (`sec min hour dom month dow year`); 5-field crontab is rejected.
- `moat gh pr reply` is moat-only (real `gh` has no such subcommand).

## Common Workflows

### Create a new project

```bash
# 1. Create repository
moat repos create --name my-app --git-url https://github.com/org/my-app

# 2. Create task (sandbox starts automatically)
moat tasks create --repo <repo-id> --name "Build landing page" --message "Create a responsive landing page"

# 3. Check task status
moat tasks status <task-id>
```

> **Note:** Creating a task automatically starts its sandbox. Use `moat tasks start` to resume a previously stopped task.

### Monitor and interact with a running task

```bash
# Check what's running
moat tasks list --status running

# Send follow-up instructions (note: `moat send --task`, NOT `moat messages send`)
moat send --task <task-id> --text "Also add a dark mode toggle"

# View conversation history
moat messages list <task-id>

# Recover full conversation context (full-text transcript)
moat messages list <task-id> --full
```

### Task-to-task messaging (parent/child coordination)

From inside a sandbox, report back to a parent task or drive a child task with
the same command:

```bash
moat send --task <other-task-id> --text "Status update or instruction"
```

The recipient agent sees the message attributed to your task id with a reply
hint baked in.

### Manage task lifecycle

```bash
moat tasks status <task-id>      # Check current state (environment, agent, servers)
moat tasks stop <task-id>        # Pause (scale to 0, preserves state)
moat tasks start <task-id>       # Resume task (provisions sandbox if needed)
moat tasks restart <task-id>     # Recreate pod (keeps PVC)
```

### GitHub operations (`moat gh`)

`moat gh` proxies `pr create`, `pr comment`, `pr reply`, and `issue comment`
through typed backend endpoints (`/api/v1/github/*`) so the task↔PR binding,
the `Moatless task:` PR-body footer, and webhook echo dedup happen as part of
the API call instead of being reconstructed from shell-command parsing. Every
other gh subcommand — or one of the typed mutations with flags the typed
parser doesn't know (`--fill`, `--body-file`, …) — passes through verbatim to
the local `gh` binary, so `moat gh` is never a dead end. (`pr reply` is
moat-only: real `gh` has no such subcommand, so its fallthrough just errors.)

```bash
# Typed (backend-proxied): -R and --head are inferred from the cwd git repo,
# --task defaults to $TASK_ID inside a sandbox
moat gh pr create --title "fix: x" --body "…"
moat gh pr comment 42 --body "Done — see latest push."
moat gh pr reply 42 123456789 --body "Good catch — fixed."
moat gh issue comment 7 --body "Triaged." -R owner/repo --task <task-id>

# Passthrough (local gh, unchanged)
moat gh pr view 42
moat gh api /user
```

### Configure adapter & trigger integrations

Setting up an external integration is **two steps**: create an adapter connection
(credentials/webhook secret), then create a trigger that routes events from that
connection into tasks on a specific repo.

Valid `--adapter` kinds: `slack`, `telegram`, `github`, `linear`, `webhook`.
Cron is **not** an adapter — it's a separate trigger kind (`triggers create-schedule`).

```bash
# Slack — route a specific channel to a repo
moat adapters connections create --adapter slack --external-account-id T0123456789
# → returns <conn-id>
moat triggers create-adapter-event \
  --adapter slack --connection-id <conn-id> \
  --source-matcher C0123456789 \
  --repository-id <repo-id> --routing-mode by_subject --agent-type claude-code

# Telegram — all DMs to a repo (wildcard matcher)
moat adapters connections create --adapter telegram --external-account-id <bot-id>
moat triggers create-adapter-event \
  --adapter telegram --connection-id <conn-id> \
  --source-matcher '*' \
  --repository-id <repo-id> --routing-mode by_subject --agent-type claude-code

# GitHub PR — each PR becomes a task (full walkthrough in soap-plugin skill)
WEBHOOK_SECRET=$(openssl rand -hex 20)
moat adapters connections create --adapter github --external-account-id <org-or-user> \
  --webhook-secret "$WEBHOOK_SECRET"
moat triggers create-adapter-event \
  --adapter github --connection-id <conn-id> \
  --source-matcher "owner/repo" \
  --repository-id <repo-id> --routing-mode by_subject --agent-type claude-code \
  --run-as-user-id <gh-connected-user-id>

# Cron — scheduled trigger (7-field cron: sec min hour dom month dow year)
# Standard 5-field crontab (e.g. "0 6 * * MON-FRI") is rejected.
moat triggers create-schedule \
  --cron-expression "0 0 6 * * MON-FRI *" \
  --timezone "Europe/Stockholm" \
  --message-template "Run daily check" \
  --repository-id <repo-id> --routing-mode by_subject --agent-type claude-code

# Listing
moat adapters connections list                    # all connections
moat adapters connections list --adapter github
moat triggers list                                # all triggers
moat triggers list --kind adapter-event --adapter slack
moat bindings list --task <task-id>               # destinations a task is bound to
```

> `routing-mode` defaults to `by_subject`. The old `moat channels defaults`
> command has no replacement — add a trigger per repo instead.

### Declare automations in the repo ("loops")

A repo can declare automations in git under `.moatless/loops/<loop-name>/LOOP.md`; `moat repos sync-config <repo-id>` reconciles them into triggers. YAML frontmatter = config, markdown body = the agent prompt. Sources: `github_pr`, `schedule`, `slack`, `linear`. Full frontmatter reference, per-source examples, and connection-resolution rules: [references/loops.md](references/loops.md).

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

### Debug a failing task

```bash
# 1. Run diagnose — aggregates status, container diagnostics, and recent errors
moat tasks diagnose <task-id>

# 2. Check previous container logs (crashed/restarted containers)
moat tasks logs <task-id> --server sandbox --previous
moat tasks logs <task-id> --server preview --previous

# 3. Stream live server logs
moat tasks logs <task-id> --server preview

# 4. Check and fix repo config if needed
moat repos get <repo-id>                           # review server config
moat repos update <repo-id> --servers-json '[...]'  # fix start commands
moat tasks restart <task-id>                        # restart with new config

# 5. View lifecycle events (sandbox + server events with errors highlighted)
moat events list <task-id> --types sandbox,server
moat events list <task-id> --errors         # show only errors

# 6. Check files in the workspace
moat tasks files <task-id> tree
moat tasks files <task-id> read --path CLAUDE.md
```

### Troubleshoot system prompt delivery

```bash
# Check what system prompt was stored on the task
moat tasks get <task-id> --json | jq -r '.appendSystemPrompt'

# Check sandbox logs for prompt delivery
kubectl logs deployment/moatless-ws-<task-id> -c sandbox | grep -E "system_prompt|Assembled system"
```

### Multi-environment usage

```bash
# Target a specific environment (default: https://moatless.dev)
MOATLESS_API_URL=https://moatless.xyz moat tasks list

# Or export for the session
export MOATLESS_API_URL=https://moatless.xyz
moat login     # Login is per-host (stored in credentials.json profiles)
moat repos list
```

### Scripting with JSON output

```bash
# Get task ID programmatically
moat tasks list --status running --json | jq '.[0].id'

# Get task environment status
moat tasks status <task-id> --json | jq '.environmentStatus'
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

## Development

```bash
# Build and lint
cargo clippy -p moatless-cli -- -D warnings

# Run without installing
cargo run -p moatless-cli -- repos list

# Install after changes
cargo install --path crates/cli
```
