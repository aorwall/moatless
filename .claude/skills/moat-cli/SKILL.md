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

## Command Quick Reference

| Command | Description | Key Flags |
|---------|-------------|-----------|
| `moat login` | Authenticate via browser OAuth | — |
| `moat logout` | Remove saved credentials | — |
| `moat whoami` | Show current user info | — |
| `moat status` | Show API URL, auth mode, profile | — |
| `moat repos list` | List all repositories | — |
| `moat repos get <id>` | Get repository details | — |
| `moat repos create` | Create a repository | `--name`, `--git-url`, `--branch`, `--provider`, `--storage`, `--storage-mode`, `--docker-storage`, `--docker-storage-mode` |
| `moat repos update <id>` | Update a repository | `--name`, `--branch`, `--git-url`, `--servers-json`, `--env-var`, `--setup-command`, `--startup-timeout`, `--storage`, `--storage-mode`, `--docker-storage`, `--docker-storage-mode` |
| `moat repos sync-config <id>` | Sync `.moatless/config.json` + `.moatless/loops/` from the repo | — |
| `moat tasks list` | List tasks | `--status`, `--limit` |
| `moat tasks get <id>` | Get task details | — |
| `moat tasks create` | Create a task | `--repo` (name or ID), `--name`, `--message`, `--parent`, `--no-parent`, `--branch` |
| `moat tasks children [id]` | List child tasks | defaults to `$TASK_ID` |
| `moat tasks start <id>` | Start a task | — |
| `moat tasks stop <id>` | Stop a task | — |
| `moat tasks close <id>` | Close a task | — |
| `moat tasks status <id>` | Show full status (environment, agent, servers) | — |
| `moat tasks restart <id>` | Restart sandbox for task | — |
| `moat tasks logs <id>` | Stream server container logs | `--server` (default: sandbox), `--previous` |
| `moat tasks diagnose <id>` | Aggregated diagnostics (status + containers + errors) | `--limit` |
| `moat tasks files <id> tree` | Show file tree with git status | `--path` |
| `moat tasks files <id> read` | Read file content from workspace | `--path` (required) |
| `moat tasks files <id> list` | List files in workspace | `--path`, `--recursive` |
| `moat messages list <id>` | List messages for a task | `--limit`, `--before-id`, `--full` (full-text transcript) |
| `moat messages reply` | Reply to a specific inbound message (backend routes to Slack thread / Linear comment / GitHub PR reply / Telegram) | `--message <id>`, `--text`, `--file` |
| `moat messages react` | React to a specific message with an emoji | `--message <id>`, `--emoji`, `--remove` |
| `moat send --task <id>` | Send a message to a task (drives the agent). There is NO `moat messages send` | `--text`, `--mode` (edit/plan), `--file` |
| `moat send --user <login>` | DM a moatless user | `--via` (slack/telegram), `--text` |
| `moat send --to <dest>` | Adapter pass-through send (e.g. `channel:C123`, `issue:LIN-12`) | `--via`, `--connection`, `--text` |
| `moat gh pr create` | Create a GitHub PR via the backend (auto task binding + footer) | `--title`/`-t`, `--body`/`-b`, `--head`, `--base`, `--draft`, `-R owner/repo`, `--task` |
| `moat gh pr comment <n>` | Comment on a PR via the backend | `--body`/`-b`, `-R`, `--task` |
| `moat gh pr reply <n> <comment_id>` | Threaded reply to a PR review comment via the backend (moat-only) | `--body`/`-b`, `-R`, `--task` |
| `moat gh issue comment <n>` | Comment on an issue via the backend | `--body`/`-b`, `-R`, `--task` |
| `moat gh <anything else>` | Passthrough to the local `gh` CLI (exit code propagated) | any gh flags |
| `moat adapters list` | List supported adapter kinds | — |
| `moat adapters connections list` | List adapter connections | `--adapter` |
| `moat adapters connections create` | Create/upsert an adapter connection | `--adapter`, `--external-account-id`, `--connection-kind`, `--webhook-secret`, `--transport-config`, `--team-id` |
| `moat triggers list` | List triggers | `--kind`, `--adapter` |
| `moat triggers create-adapter-event` | Webhook-driven trigger | `--adapter`, `--connection-id`, `--source-matcher`, `--repository-id`, `--routing-mode`, `--agent-type`, `--prompt`, `--task-name`, `--tag`, `--run-as-user-id`, `--event-filter`, `--adapter-config`, `--allow-direct-send`, `--close-task-on-close`, `--notify-on-success`, `--name`, `--team-id` |
| `moat triggers create-schedule` | Cron-driven trigger | `--cron-expression`, `--timezone`, `--message-template`, `--repository-id`, `--routing-mode`, `--agent-type`, `--prompt`, `--task-name`, `--tag`, `--run-as-user-id`, `--name`, `--team-id` |
| `moat triggers update --id <id>` | Update a trigger (paired `--clear-*` flags) | many — see source |
| `moat triggers delete --id <id>` | Delete a trigger | `--yes` |
| `moat bindings list --task <id>` | List task bindings | — |
| `moat bindings create` | Create a task binding | `--task`, `--adapter`, `--connection-id`, `--subject-kind`, `--subject-key`, `--trigger-id`, `--destination-config`, `--metadata` |
| `moat teams list` | List teams | — |
| `moat teams get <id>` | Get team details | — |
| `moat teams settings get <id>` | Show team settings | — |
| `moat teams settings set <id>` | Update team settings | `--default-repo` |
| `moat settings get` | Show current user settings | — |
| `moat settings set` | Update user settings | `--default-repo` |
| `moat events list <id>` | List task lifecycle events | `--types`, `--limit`, `--errors`, `--follow` |
| `moat commands` | List available commands | — |
| `moat tags` | List tags | — |

For full flag details and examples, see [Command Reference](references/command-reference.md).

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

Instead of creating triggers by hand, a repo can declare its automations in git
under `.moatless/loops/<loop-name>/LOOP.md` and have `sync-config` reconcile them
into the trigger tables. The YAML **frontmatter is the config**; the markdown
**body is the prompt** (the agent's `append_system_prompt`) — mirroring Claude
Code's `SKILL.md`.

```markdown
---
on: github_pr            # event source; also: schedule, slack, linear
agentType: claude-code   # optional, default claude-code
routingMode: by_subject  # optional, default by_subject
tags: [review]           # optional
# optional: name, runAsUserId, sourceMatcher, teamId, eventFilter, adapterConfig, active, inactiveTtlDays
---

Review this pull request using the java-review skill. Post inline comments.
```

```markdown
---
on: schedule
schedule:
  cron: "0 0 6 * * MON-FRI *"     # 7-field cron (sec min hour dom mon dow year)
  timezone: Europe/Stockholm
  message: "Run the daily platform health check."
---

Run the daily-health-check skill and file a Linear ticket for any regressions.
```

```markdown
---
on: slack
sourceMatcher: C0000000001      # the channel id to listen on (required for slack)
teamId: T0000000001             # Slack workspace id; omit if only one slack connection exists
routingMode: ongoing            # one long-lived task instead of one-per-subject
adapterConfig:
  allow_bot_messages: true
  trigger: all_messages
---

Monitor this channel for CI failures and triage them in-thread.
```

```markdown
---
on: linear
sourceMatcher: <linear-team-uuid>   # the Linear TEAM whose issues to watch (required)
teamId: <linear-org-id>             # Linear workspace/org id; omit if only one linear connection exists
triggerStates: [Ready]              # spawn a task when an issue ENTERS one of these state names
startState: In Progress             # optional — move the issue to this state when the task is picked up
projectIds: [<project-uuid>]        # optional — only fire for issues in these projects
requireLabels: [agent]              # optional — issue must carry all of these labels
---

Pick up this Linear ticket, implement it, and open a PR linked back to the issue.
```

```bash
moat repos sync-config <repo-id>                  # fetch + reconcile loops
moat triggers list --repository-id <repo-id>      # synced loops show up here
```

Notes:
- Synced loops are tracked as triggers with `managed_by = 'repo_loop'`, keyed by
  `(repository, loop dir name)`. Re-running `sync-config` upserts them and removes
  loops whose directory was deleted. Manually-created triggers are never touched.
- `github_pr` loops need a pre-existing GitHub **connection** for the org (it
  holds the webhook secret and can't be committed to git). With no connection the
  loop is **skipped** with a reason, not errored. Schedule loops need no connection.
- `slack` loops resolve their **connection** from `teamId` (the Slack workspace id,
  e.g. `T0000000001`); if `teamId` is omitted and exactly one slack connection
  exists it's used, otherwise the loop is **skipped** with a reason. `sourceMatcher`
  (the channel id) is **required** — there's no `owner/repo` default. Pair with
  `routingMode: ongoing` for a single long-lived triage task.
- `linear` loops listen for **issue status transitions** and spawn a task when an
  issue ENTERS one of `triggerStates` (matched against Linear's state **names**).
  This is opt-in: the synced trigger sets `create_on_status_entry: true`
  automatically, so existing mention/assignment Linear rules are unaffected.
  `sourceMatcher` is the Linear **team UUID** (the listen scope) and is
  **required** — there's no `owner/repo` default. The **connection** resolves from
  `teamId` (the Linear **workspace/organization id**); omit it when exactly one
  Linear connection is installed, otherwise the loop is **skipped**. `projectIds`
  and `requireLabels` further narrow which issues fire. `startState` (optional)
  moves the issue into that workflow state (e.g. `In Progress`) when the task is
  picked up, so the board reflects the agent took it — resolved within the
  issue's team and skipped if the issue is already there. When the issue reaches
  a terminal state (`completed`/`canceled`) the spawned task is auto-closed.
  The same knobs work on a hand-made trigger:
  `moat triggers create-adapter-event --adapter linear --connection-id <id>
  --source-matcher <team-uuid> --adapter-config
  '{"create_on_status_entry":true,"trigger_states":["Ready"],"project_ids":["<proj>"],"start_state":"In Progress"}'`.
- `runAsUserId` resolves: frontmatter → `MOATLESS_DEFAULT_RUN_AS_USER_ID` →
  repo owner → repo creator.

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
| `commands/tasks.rs` | Task subcommands (includes status, restart, logs, files) |
| `commands/repos.rs` | Repository subcommands |
| `commands/messages.rs` | Message subcommands |
| `commands/events.rs` | Event subcommands (list, follow) |
| `commands/adapters.rs` | Adapter, trigger, and binding subcommands |
| `commands/gh.rs` | GitHub proxy (`gh pr create`/`comment`/`reply` typed, rest passthrough) |
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
