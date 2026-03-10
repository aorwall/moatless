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

| Flag | Description |
|------|-------------|
| `--name` | New name |
| `--branch` | New default branch |
| `--git-url` | New git URL |

- **API**: `PUT /api/v1/repositories/{repo_id}`

```bash
moat repos update 3c9b49b9-... --name new-name
moat repos update 3c9b49b9-... --branch develop
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

## Channel Commands

### `moat channels list`

List all channel configurations.

- **API**: `GET /api/v1/channels/configs`
- **Table columns**: ID, PROVIDER, SOURCE, MODE

### `moat channels create`

Create a channel configuration.

| Flag | Required | Description |
|------|----------|-------------|
| `--provider` | Yes | Channel provider: `slack`, `telegram`, `github_pr` |
| `--source` | Yes | Source identifier (see per-provider details below) |
| `--repo` | Yes | Repository ID to route messages to |
| `--mode` | Yes | Routing mode: `ongoing`, `per_message`, `thread_based` |
| `--provider-config` | No | Provider-specific config as JSON object |

- **API**: `POST /api/v1/channels/configs`

#### Slack

Source ID is the Slack channel ID (e.g., `C0123456789`).

```bash
# Basic — mention-only trigger (default)
moat channels create --provider slack --source C0123456789 --repo <repo-id> --mode per_message

# All messages trigger
moat channels create --provider slack --source C0123456789 --repo <repo-id> --mode per_message \
  --provider-config '{"trigger":"all_messages"}'
```

#### Telegram

Source ID is the chat ID (numeric, e.g., `-1001234567890`) or `*` for all DMs.

```bash
# Specific group chat
moat channels create --provider telegram --source -1001234567890 --repo <repo-id> --mode thread_based

# All DMs (wildcard)
moat channels create --provider telegram --source '*' --repo <repo-id> --mode ongoing \
  --provider-config '{"bot_username":"@mybot","secret_token":"my-webhook-secret"}'
```

**Provider config fields** (`--provider-config` JSON):

| Field | Type | Description |
|-------|------|-------------|
| `secret_token` | string | Webhook verification token (X-Telegram-Bot-Api-Secret-Token) |
| `bot_username` | string | Bot's @username for filtering self-replies |

#### GitHub PR

Source ID is the repository in `owner/repo` format. Each PR becomes a separate thread. Recommended routing mode: `thread_based`.

```bash
# Basic — includes PR diff on open (default)
moat channels create --provider github_pr --source aorwall/my-repo --repo <repo-id> --mode thread_based

# With bot token and webhook secret
moat channels create --provider github_pr --source aorwall/my-repo --repo <repo-id> --mode thread_based \
  --provider-config '{"bot_token":"ghp_abc123","webhook_secret":"whsec_xyz","bot_username":"my-bot[bot]"}'

# Disable diff inclusion
moat channels create --provider github_pr --source aorwall/my-repo --repo <repo-id> --mode thread_based \
  --provider-config '{"include_diff":false}'
```

**Provider config fields** (`--provider-config` JSON):

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `webhook_secret` | string | — | Per-channel webhook secret (overrides `GITHUB_CHANNEL_WEBHOOK_SECRET` env) |
| `bot_token` | string | — | GitHub token for API calls (overrides `GITHUB_CHANNEL_TOKEN` env, falls back to owner's linked GitHub token) |
| `bot_username` | string | — | Bot's GitHub login (filters self-replies from webhooks) |
| `include_diff` | bool | `true` | Include PR diff when a new PR is opened |
| `max_diff_size` | number | `10000` | Max diff characters to include |
| `events_filter` | string[] | `[]` (all) | Filter event types: `"pull_request"`, `"issue_comment"`, `"pull_request_review_comment"`, `"pull_request_review"` |

**Webhook URL**: Configure your GitHub repository webhook to point to `{API_URL}/api/v1/channels/github-pr/webhook` with content type `application/json`.

### `moat channels defaults get`

Get default settings for a channel provider. Useful for Slack and Telegram where wildcard routing applies. Not applicable to GitHub PR (always specific `owner/repo`).

```bash
moat channels defaults get --provider slack
moat channels defaults get --provider telegram --json
```

### `moat channels defaults set`

Set default settings for a channel provider (creates or updates a wildcard config).

| Flag | Required | Description |
|------|----------|-------------|
| `--provider` | Yes | Channel provider name |
| `--repo` | Yes | Default repository ID |
| `--mode` | Yes | Default routing mode |
| `--provider-config` | No | Default provider config JSON |
| `--team` | No | Scope defaults to a team |

```bash
moat channels defaults set --provider slack --repo <repo-id> --mode per_message \
  --provider-config '{"trigger":"mention_only"}'

moat channels defaults set --provider telegram --repo <repo-id> --mode ongoing --team <team-id>
```

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
- **Output**: Default channel repository ID

```bash
moat teams settings get <team-id>
moat teams settings get <team-id> --json
```

### `moat teams settings set <team_id>`

Update a team's settings. Requires team membership or admin role.

| Flag | Required | Description |
|------|----------|-------------|
| `--default-repo` | No | Default repository ID for channel integrations |

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
- **Output**: Default channel repository ID

```bash
moat settings get
moat settings get --json
```

### `moat settings set`

Update user settings.

| Flag | Required | Description |
|------|----------|-------------|
| `--default-repo` | No | Default repository ID for channel integrations (e.g., Telegram auto-routing) |

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
| `sandbox status` | GET | `/api/sandbox/v1/tasks/{id}/status` |
| `sandbox start` | POST | `/api/sandbox/v1/tasks/{id}/start` |
| `sandbox stop` | POST | `/api/sandbox/v1/tasks/{id}/stop` |
| `sandbox restart` | POST | `/api/sandbox/v1/tasks/{id}/restart` |
| `messages list` | GET | `/api/v1/tasks/{id}/messages` |
| `messages send` | POST | `/api/v1/tasks/{id}/messages` |
| `channels list` | GET | `/api/v1/channels/configs` |
| `channels create` | POST | `/api/v1/channels/configs` |
| `channels defaults get` | GET | `/api/v1/channels/defaults/{provider}` |
| `channels defaults set` | PUT | `/api/v1/channels/defaults/{provider}` |
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
