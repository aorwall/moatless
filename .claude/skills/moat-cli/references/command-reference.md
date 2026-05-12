# Moat CLI Command Reference

Complete reference for all `moat` CLI commands, flags, and API mappings.

## Authentication Commands

### `moat login`

Authenticate with the Moatless API via browser-based OAuth.

- Starts a local HTTP server on a random port
- Opens browser to `{API_URL}/api/v1/auth/cli/login?port={port}`
- Receives callback with API key (`mvk_...`) and user login
- Saves credentials to `~/.config/moatless/credentials.json`

```bash
moat login
# → Opening browser for authentication...
# → Successfully logged in as albert
```

### `moat logout`

Remove saved credentials for the current API host.

```bash
moat logout
# → Logged out from moatless.dev
```

### `moat whoami`

Display current authenticated user info.

- **API**: `GET /api/v1/auth/me`
- **Output**: User ID, login, name, email, role

```bash
moat whoami
# → User: albert (Albert Example)
# → Email: albert@example.com
# → Role: admin
```

### `moat status`

Show current configuration context (no API call).

- **Output**: API URL, auth mode (standalone/sandbox), profile name

```bash
moat status
# → API URL: https://moatless.dev
# → Auth mode: standalone
# → Profile: moatless.dev
```

---

## Repository Commands

### `moat repos list`

List all repositories.

- **API**: `GET /api/v1/repositories`
- **Table columns**: ID, NAME, PROVIDER, BRANCH

```bash
moat repos list
moat repos list --json
```

### `moat repos get <repo_id>`

Get details for a specific repository.

- **API**: `GET /api/v1/repositories/{repo_id}`

```bash
moat repos get 3c9b49b9-e9a9-459b-905f-55617b677ffd
```

### `moat repos create`

Create a new repository.

| Flag | Required | Default | Description |
|------|----------|---------|-------------|
| `--name` | Yes | — | Repository name |
| `--git-url` | Yes | — | Git clone URL |
| `--branch` | No | `main` | Default branch |
| `--provider` | No | `github` | Git provider |

- **API**: `POST /api/v1/repositories`

```bash
moat repos create --name my-app --git-url https://github.com/org/my-app
moat repos create --name my-app --git-url https://github.com/org/my-app --branch develop --provider github
```

### `moat repos update <repo_id>`

Update an existing repository. All flags are optional — only provided fields are updated.

| Flag | Type | Description |
|------|------|-------------|
| `--name` | `String` | New name |
| `--branch` | `String` | New default branch |
| `--git-url` | `String` | New git URL |
| `--docker-image` | `String` | Docker image for sandbox |
| `--setup-command <CMD>` | repeatable | Setup commands run before server starts |
| `--server-start-command <CMD>` | repeatable | Legacy server start commands |
| `--env-var <KEY=VALUE>` | repeatable | Repo-level environment variables |
| `--startup-timeout <SECS>` | `i32` | Startup timeout in seconds |
| `--servers-json <JSON>` | `String` | Full servers config as JSON string |
| `--resource-cpu-request` | `String` | CPU request (e.g. "200m") |
| `--resource-cpu-limit` | `String` | CPU limit (e.g. "4") |
| `--resource-memory-request` | `String` | Memory request (e.g. "2560Mi") |
| `--resource-memory-limit` | `String` | Memory limit (e.g. "4096Mi") |

- **API**: `PUT /api/v1/repositories/{repo_id}`

```bash
# Basic updates
moat repos update 3c9b49b9-... --name new-name
moat repos update 3c9b49b9-... --branch develop

# Update server config (JSON array of ServerConfig objects)
moat repos update <repo-id> --servers-json '[{"name":"preview","port":3000,"startCommands":["bunx next dev --port 3000"],"default":true}]'

# Set environment variables
moat repos update <repo-id> --env-var PORT=3000 --env-var NODE_ENV=development

# Set setup commands
moat repos update <repo-id> --setup-command "bun install" --setup-command "bun run build"

# Set startup timeout
moat repos update <repo-id> --startup-timeout 120
```

---

## Task Commands

### `moat tasks list`

List tasks with optional filtering.

| Flag | Default | Description |
|------|---------|-------------|
| `--status` | — | Filter by status (e.g., `running`, `completed`, `created`) |
| `--limit` | 50 | Maximum number of results |

- **API**: `GET /api/v1/tasks?limit={limit}&status={status}`
- **Table columns**: TASK_ID, STATUS, TITLE, REPO

```bash
moat tasks list
moat tasks list --status running
moat tasks list --status completed --limit 10
```

### `moat tasks get <task_id>`

Get detailed info for a specific task.

- **API**: `GET /api/v1/tasks/{task_id}`

```bash
moat tasks get abc123
```

### `moat tasks create`

Create a new task.

| Flag | Required | Default | Description |
|------|----------|---------|-------------|
| `--repo` | Yes | — | Repository ID |
| `--name` | Yes | — | Task name/title |
| `--message` | No | — | Initial message/instructions |
| `--parent` | No | `$TASK_ID` env | Parent task ID |
| `--no-parent` | No | — | Explicitly create without parent |
| `--branch` | No | — | Git branch name |

- **API**: `POST /api/v1/tasks`

**Auto-parent behavior**: If `TASK_ID` env var is set and `--no-parent` is not specified, the new task automatically gets `TASK_ID` as its parent. This enables hierarchical task trees inside sandbox pods.

```bash
# Basic creation
moat tasks create --repo <repo-id> --name "Build feature X"

# With initial instructions
moat tasks create --repo <repo-id> --name "Fix bug" --message "The login form crashes on submit"

# Explicit parent
moat tasks create --repo <repo-id> --name "Subtask" --parent <parent-task-id>

# No parent (override $TASK_ID)
moat tasks create --repo <repo-id> --name "Independent task" --no-parent
```

### `moat tasks children [task_id]`

List child tasks of a parent.

- If `task_id` omitted, uses `$TASK_ID` env var
- **API**: `GET /api/v1/tasks?parentTaskId={parent}`

```bash
moat tasks children                    # uses $TASK_ID
moat tasks children <parent-task-id>   # explicit parent
```

### `moat tasks start <task_id>`

Start a task.

- **API**: `POST /api/v1/tasks/{task_id}/start`

### `moat tasks stop <task_id>`

Stop a running task.

- **API**: `POST /api/v1/tasks/{task_id}/stop`

### `moat tasks close <task_id>`

Close/complete a task.

- **API**: `POST /api/v1/tasks/{task_id}/close`

### `moat tasks diagnose <task_id>`

Aggregated sandbox diagnostics: status, container states, and recent error events in a single view.

| Flag | Default | Description |
|------|---------|-------------|
| `--limit` | 10 | Number of recent error events to show |

- **API**: Multiple — aggregates status, container diagnostics, and events
- **Output**: Environment status, container restart counts, recent errors

```bash
moat tasks diagnose <task-id>
moat tasks diagnose <task-id> --limit 20
moat tasks diagnose <task-id> --json
```

### `moat tasks logs <task_id>`

Stream server container logs from the sandbox pod.

| Flag | Default | Description |
|------|---------|-------------|
| `--server` / `-s` | `sandbox` | Container name to stream logs from (e.g. `sandbox`, `preview`, `files-server`) |
| `--previous` | `false` | Fetch logs from the previous (crashed/restarted) container instance |

- **API**: `GET /api/sandbox/v1/tasks/{task_id}/servers/{server}/logs[?previous=true]`

```bash
# Stream sandbox logs (default)
moat tasks logs <task-id>

# Stream preview server logs
moat tasks logs <task-id> --server preview

# View previous container logs (after crash/restart)
moat tasks logs <task-id> --server sandbox --previous
moat tasks logs <task-id> --server preview --previous
```

---

## Sandbox Commands

All sandbox commands use the sandbox API routes (`/api/sandbox/v1/`) and authenticate with `X-User-Token` header when in sandbox mode.

### `moat sandbox status <task_id>`

Show sandbox environment and agent status.

- **API**: `GET /api/sandbox/v1/tasks/{task_id}/status`
- **Output**: Environment status, agent state, server list with ports

```bash
moat sandbox status <task-id>
moat sandbox status <task-id> --json
```

### `moat sandbox start <task_id>`

Start the sandbox for a task. If no sandbox exists, auto-provisions one.

- **API**: `POST /api/sandbox/v1/tasks/{task_id}/start`

### `moat sandbox stop <task_id>`

Stop the sandbox (scales Deployment to 0, preserves PVC/state).

- **API**: `POST /api/sandbox/v1/tasks/{task_id}/stop`

### `moat sandbox restart <task_id>`

Restart the sandbox (deletes pods, Deployment recreates them).

- **API**: `POST /api/sandbox/v1/tasks/{task_id}/restart`

---

## Message Commands

### `moat messages list <task_id>`

List all messages for a task.

- **API**: `GET /api/v1/tasks/{task_id}/messages`
- **Table columns**: ROLE, CONTENT (truncated to 80 chars), CREATED

```bash
moat messages list <task-id>
moat messages list <task-id> --json   # Full message content
```

### `moat messages send <task_id>`

Send a message to a task.

| Flag | Required | Description |
|------|----------|-------------|
| `--text` | Yes | Message content |

- **API**: `POST /api/v1/tasks/{task_id}/messages`
- **Body**: `{"message": "{text}"}`

```bash
moat messages send <task-id> --text "Add error handling to the login form"
```

---

## Adapter, Trigger, and Binding Commands

The legacy `moat channels` command group has been removed. External integrations
are now split across three orthogonal CLI groups:

- **adapters** — register credentials/webhook-secrets for an external system (Slack workspace, GitHub org, etc.)
- **triggers** — declare what should happen when events arrive (route a webhook to a repo, or fire on a cron schedule)
- **bindings** — connect a specific task to a specific destination (used by `moat reply`)

Adapter kinds (`--adapter`): `slack`, `telegram`, `github`, `linear`, `webhook`.
Cron is **not** an adapter — use `triggers create-schedule`.

### `moat adapters list`

List adapter kinds supported by the backend.

### `moat adapters connections list`

List adapter connections.

| Flag | Required | Description |
|------|----------|-------------|
| `--adapter` | No | Filter by adapter kind |

### `moat adapters connections create`

Create or upsert an adapter connection. The same `external_account_id` upserts —
re-running with the same value updates the existing connection.

| Flag | Required | Description |
|------|----------|-------------|
| `--adapter` | Yes | Adapter kind |
| `--external-account-id` | Yes | Workspace/org/account ID at the external system (e.g. Slack team ID, GitHub org) |
| `--connection-kind` | No | Adapter-specific subtype (e.g. `oauth`, `personal_access_token`) |
| `--webhook-secret` | No | Secret used to verify inbound webhooks for this connection |
| `--transport-config` | No | Adapter-specific transport JSON (e.g. base URL, OAuth tokens) |
| `--team-id` | No | Scope the connection to a team |

```bash
# Slack workspace
moat adapters connections create --adapter slack --external-account-id T0123456789

# GitHub org with webhook secret
WEBHOOK_SECRET=$(openssl rand -hex 20)
moat adapters connections create --adapter github \
  --external-account-id soaplabs \
  --webhook-secret "$WEBHOOK_SECRET"
```

### `moat triggers list`

List triggers, optionally filtered.

| Flag | Required | Description |
|------|----------|-------------|
| `--kind` | No | `adapter-event` or `schedule` |
| `--adapter` | No | Filter adapter-event triggers by adapter kind |

### `moat triggers create-adapter-event`

Create a trigger that fires when a webhook arrives via a given adapter connection
and routes matching events into a task on the chosen repo.

| Flag | Required | Description |
|------|----------|-------------|
| `--adapter` | Yes | Adapter kind |
| `--connection-id` | Yes | Adapter connection ID (from `adapters connections create`) |
| `--source-matcher` | Yes | Matcher for the source/subject (e.g. Slack channel ID `C0123456789`, GitHub `owner/repo`, or `*` wildcard) |
| `--source-name` | No | Human-readable source name |
| `--repository-id` | Yes | Repo to route events into |
| `--routing-mode` | No | Default `by_subject` |
| `--agent-type` | No | Default `claude-code` |
| `--prompt` | No | Prompt template appended to the agent prompt |
| `--task-name` | No | Task name template |
| `--tag` | No | Tag ID (repeatable) |
| `--run-as-user-id` | No | Attribute trigger-created tasks to this user; required for GitHub triggers so PR replies use that user's GitHub token |
| `--event-filter` | No | Adapter-specific event-filter JSON |
| `--adapter-config` | No | Extra adapter-specific JSON |
| `--allow-direct-send` | No | Bool — allow direct (non-task) outbound messages |
| `--close-task-on-close` | No | Bool — close the task when the upstream subject closes (default true) |
| `--notify-on-success` | No | Bool — emit a notification when the agent finishes |
| `--name` | No | Trigger display name |
| `--team-id` | No | Scope to a team |

```bash
# Slack channel → repo
moat triggers create-adapter-event \
  --adapter slack --connection-id <conn-id> \
  --source-matcher C0123456789 \
  --repository-id <repo-id> --routing-mode by_subject --agent-type claude-code

# GitHub PRs from a repo → repo, attributed to moatless-soap
moat triggers create-adapter-event \
  --adapter github --connection-id <conn-id> \
  --source-matcher "soaplabs/my-service" \
  --repository-id <repo-id> --routing-mode by_subject --agent-type claude-code \
  --run-as-user-id 225125419 \
  --name "PR review – soaplabs/my-service"
```

### `moat triggers create-schedule`

Create a cron-driven trigger that fires on a schedule.

| Flag | Required | Description |
|------|----------|-------------|
| `--cron-expression` | Yes | 7-field cron: `sec min hour dom month dow year` (5-field crontab is rejected) |
| `--timezone` | No | IANA TZ (e.g. `Europe/Stockholm`) |
| `--message-template` | Yes | Message text sent to the agent on each tick |
| `--repository-id` | Yes | Repo to route the scheduled task into |
| `--routing-mode` | No | Default `by_subject` |
| `--agent-type` | No | Default `claude-code` |
| `--prompt`, `--task-name`, `--tag`, `--run-as-user-id`, `--name`, `--team-id` | No | Same semantics as `create-adapter-event` |

```bash
moat triggers create-schedule \
  --cron-expression "0 0 6 * * MON-FRI *" \
  --timezone "Europe/Stockholm" \
  --message-template "Run daily check" \
  --repository-id <repo-id> --routing-mode by_subject --agent-type claude-code
```

### `moat triggers update`

Update base fields, rule, or source on an existing trigger. Most string fields
have a paired `--clear-<field>` flag for explicitly clearing them.

| Flag | Required | Description |
|------|----------|-------------|
| `--id` | Yes | Trigger ID |
| `--name` / `--clear-name` | No | Update or clear the name |
| `--active` | No | Bool — enable/disable |
| `--repository-id`, `--routing-mode`, `--ongoing-task-id` / `--clear-ongoing-task-id` | No | Rule fields |
| `--prompt` / `--clear-prompt`, `--task-name` / `--clear-task-name`, `--tag` / `--clear-tags` | No | Rule fields |
| `--run-as-user-id` / `--clear-run-as-user-id`, `--agent-type` | No | Rule fields |
| `--connection-id`, `--source-matcher`, `--source-name` / `--clear-source-name` | No | Adapter-event source fields |
| `--event-filter`, `--adapter-config` / `--clear-adapter-config` | No | Adapter-event source fields |
| `--allow-direct-send`, `--close-task-on-close`, `--notify-on-success` | No | Adapter-event source fields |
| `--cron-expression`, `--timezone`, `--message-template` | No | Schedule source fields |
| `--scope`, `--team-id` / `--clear-team-id` | No | Scope (`global`, `team`, `user`) and team scoping |

Adapter-event and schedule source fields cannot be set in the same invocation.

### `moat triggers delete`

Delete a trigger. Requires `--yes` in non-interactive contexts; otherwise prompts.

| Flag | Required | Description |
|------|----------|-------------|
| `--id` | Yes | Trigger ID |
| `--yes` | No | Skip the interactive confirmation |

### `moat bindings list`

List task bindings (destinations the task replies into).

| Flag | Required | Description |
|------|----------|-------------|
| `--task` | Yes | Task ID |

### `moat bindings create`

Create a task binding. Most bindings are created automatically when a trigger
spawns a task — manual creation is mainly for testing or seeding state.

| Flag | Required | Description |
|------|----------|-------------|
| `--task` | Yes | Task ID |
| `--adapter` | Yes | Adapter kind |
| `--connection-id` | Yes | Adapter connection ID |
| `--subject-kind` | Yes | Subject type (e.g. `pr`, `thread`, `channel`) |
| `--subject-key` | Yes | Subject key (e.g. `owner/repo#42`) |
| `--trigger-id` | No | Trigger that created the binding |
| `--destination-config` | No | Adapter-specific destination JSON |
| `--metadata` | No | Free-form metadata JSON |

### GitHub PR webhook

The backend still listens at `{API_URL}/api/v1/channels/github-pr/webhook`
(the path retains the legacy `channels` segment server-side; this is not a CLI
concern). The webhook secret lives on the **adapter connection**
(`--webhook-secret` on `adapters connections create`) — one connection can serve
many triggers as long as they share that secret.

---

## Team Commands

### `moat teams list`

List all teams the current user has access to.

- **API**: `GET /api/v1/teams`
- **Table columns**: ID, NAME, SLUG, DEFAULT

```bash
moat teams list
moat teams list --json
```

### `moat teams get <team_id>`

Get details for a specific team.

- **API**: `GET /api/v1/teams/{team_id}`

```bash
moat teams get <team-id>
```

### `moat teams settings get <team_id>`

Show a team's settings.

- **API**: `GET /api/v1/teams/{team_id}/settings`
- **Output**: Default repository ID

```bash
moat teams settings get <team-id>
moat teams settings get <team-id> --json
```

### `moat teams settings set <team_id>`

Update a team's settings. Requires team membership or admin role.

| Flag | Required | Description |
|------|----------|-------------|
| `--default-repo` | No | Default repository ID for trigger-created tasks |

- **API**: `PATCH /api/v1/teams/{team_id}/settings`
- **Body**: `{"defaultChannelRepositoryId": "{repo-id}"}`
- **Validation**: The backend verifies the repository ID exists before saving

```bash
moat teams settings set <team-id> --default-repo <repo-id>
```

---

## Settings Commands (User)

### `moat settings get`

Show current user settings.

- **API**: `GET /api/v1/users/me/settings`
- **Output**: Default repository ID

```bash
moat settings get
moat settings get --json
```

### `moat settings set`

Update user settings.

| Flag | Required | Description |
|------|----------|-------------|
| `--default-repo` | No | Default repository ID for trigger-created tasks (e.g., Telegram auto-routing) |

- **API**: `PATCH /api/v1/users/me/settings`
- **Body**: `{"defaultChannelRepositoryId": "{repo-id}"}`
- **Validation**: The backend verifies the repository ID exists before saving

```bash
moat settings set --default-repo <repo-id>
```

---

## Utility Commands

### `moat commands`

List available backend commands.

- **API**: `GET /api/v1/commands`

### `moat tags`

List available tags.

- **API**: `GET /api/v1/tags`

---

## API Endpoint Mapping

| CLI Command | HTTP Method | API Path |
|-------------|------------|----------|
| `repos list` | GET | `/api/v1/repositories` |
| `repos get` | GET | `/api/v1/repositories/{id}` |
| `repos create` | POST | `/api/v1/repositories` |
| `repos update` | PUT | `/api/v1/repositories/{id}` |
| `tasks list` | GET | `/api/v1/tasks` |
| `tasks get` | GET | `/api/v1/tasks/{id}` |
| `tasks create` | POST | `/api/v1/tasks` |
| `tasks children` | GET | `/api/v1/tasks?parentTaskId={id}` |
| `tasks start` | POST | `/api/v1/tasks/{id}/start` |
| `tasks stop` | POST | `/api/v1/tasks/{id}/stop` |
| `tasks close` | POST | `/api/v1/tasks/{id}/close` |
| `tasks diagnose` | GET (multiple) | Aggregates status, diagnostics, events |
| `tasks logs` | GET | `/api/sandbox/v1/tasks/{id}/servers/{server}/logs` |
| `sandbox status` | GET | `/api/sandbox/v1/tasks/{id}/status` |
| `sandbox start` | POST | `/api/sandbox/v1/tasks/{id}/start` |
| `sandbox stop` | POST | `/api/sandbox/v1/tasks/{id}/stop` |
| `sandbox restart` | POST | `/api/sandbox/v1/tasks/{id}/restart` |
| `messages list` | GET | `/api/v1/tasks/{id}/messages` |
| `messages send` | POST | `/api/v1/tasks/{id}/messages` |
| `adapters list` | GET | `/api/v1/adapters` |
| `adapters connections list` | GET | `/api/v1/adapters/connections` |
| `adapters connections create` | POST | `/api/v1/adapters/connections` |
| `triggers list` | GET | `/api/v1/triggers` |
| `triggers create-adapter-event` | POST | `/api/v1/triggers` |
| `triggers create-schedule` | POST | `/api/v1/triggers` |
| `triggers update` | PUT | `/api/v1/triggers/{id}` |
| `triggers delete` | DELETE | `/api/v1/triggers/{id}` |
| `bindings list` | GET | `/api/v1/tasks/{task_id}/bindings` |
| `bindings create` | POST | `/api/v1/tasks/{task_id}/bindings` |
| `teams list` | GET | `/api/v1/teams` |
| `teams get` | GET | `/api/v1/teams/{id}` |
| `teams settings get` | GET | `/api/v1/teams/{id}/settings` |
| `teams settings set` | PATCH | `/api/v1/teams/{id}/settings` |
| `settings get` | GET | `/api/v1/users/me/settings` |
| `settings set` | PATCH | `/api/v1/users/me/settings` |
| `commands` | GET | `/api/v1/commands` |
| `tags` | GET | `/api/v1/tags` |
| `whoami` | GET | `/api/v1/auth/me` |

## Error Response Format

All API errors return consistent JSON:

```json
{"error": "Human-readable error message"}
```

The CLI extracts the `error` field and prints it to stderr. Use `--verbose` to see full HTTP request/response details for debugging.

## Credential File Format

Location: `~/.config/moatless/credentials.json`

```json
{
  "default": "moatless.dev",
  "profiles": {
    "moatless.dev": {
      "api_key": "mvk_...",
      "user_login": "albert",
      "created_at": "2025-03-08T12:00:00Z"
    }
  }
}
```

Profiles are keyed by API hostname (with port if non-standard). The `default` field points to the active profile. File permissions are set to `0600` on Unix.
