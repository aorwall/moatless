# API Authentication Guide

This guide explains how to authenticate with the Moatless API.

There are two credentials: a **session cookie** (browsers and the CLI login
flow) and an **API key** (programmatic access). Every API route requires one or
the other — there is no unauthenticated mode.

## Password login

When the server runs local username/password auth (`AUTH_PROVIDER` unset or
`local`), `POST /api/v1/auth/password/login` exchanges credentials for a session
cookie:

```bash
curl -i -X POST -H "Content-Type: application/json" \
  -d '{"email":"admin@example.com","password":"your-password"}' \
  "$BASE_URL/api/v1/auth/password/login"
# → 200 + Set-Cookie: moatless_session=...
```

`GET /api/v1/auth/mode` reports which flow a deployment runs (`password` or
`oauth`) and is public, so a login page can call it before authenticating.

The bootstrap admin account is seeded at startup from `ADMIN_EMAIL` and
`ADMIN_PASSWORD`. When the password is unset, a random one is generated on
first boot and logged **once** at WARN. Change your own with
`PUT /api/v1/auth/password` (session cookie only), or reset another user's with
`PUT /api/v1/users/{login}/password` as an admin.

On OAuth deployments the password endpoints answer 404 and `/api/v1/auth/login`
redirects to the configured provider instead.

## Creating an API key

API keys have the format `mvk_<random_string>`. The simplest way to get one is
the CLI:

```bash
MOATLESS_API_URL="$BASE_URL" moat login
```

`moat login` signs in (password prompt or browser OAuth), mints a key named
`moat-cli` and stores it in `~/.config/moatless/credentials.json`.

A signed-in session can also mint keys directly with
`POST /api/v1/auth/api-keys`, body `{"name": "...", "expiresInDays": 30}`
(`expiresInDays` is optional). The response carries the full key in `key`; it
is only returned once, so store it securely.

## Using the API key

Include the API key in the `Authorization` header with the `Bearer` scheme:

```bash
curl -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/repositories"
```

The response is a JSON array of repositories, each with an `id`, `name` and
`kind`.

## Selected endpoints

The backend serves its full OpenAPI document at `GET /openapi.json` (it also
needs a credential). The endpoints most scripts need:

### Repositories and Workspaces

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/repositories` | List repositories |
| POST | `/api/v1/repositories` | Register a repository |
| GET | `/api/v1/repositories/{id}` | Get repository details |
| PUT | `/api/v1/repositories/{id}` | Update a repository |
| DELETE | `/api/v1/repositories/{id}` | Delete a repository |
| GET | `/api/v1/workspaces` | List Workspaces |

### Tasks

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/tasks` | List tasks (`items` plus `pagination`) |
| POST | `/api/v1/tasks` | Create a task |
| GET | `/api/v1/tasks/{taskId}` | Get task details |
| PATCH | `/api/v1/tasks/{taskId}` | Update a task |
| DELETE | `/api/v1/tasks/{taskId}` | Delete a task |
| POST | `/api/v1/tasks/{taskId}/start` | Start a task's sandbox |
| POST | `/api/v1/tasks/{taskId}/stop` | Stop a task's sandbox |
| POST | `/api/v1/tasks/{taskId}/messages` | Send a message to the agent |
| GET | `/api/v1/tasks/{taskId}/messages` | Get task messages |

### API keys

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/auth/api-keys` | List your API keys |
| POST | `/api/v1/auth/api-keys` | Create a new API key |
| DELETE | `/api/v1/auth/api-keys/{keyId}` | Revoke an API key |

## Create and run a task

```bash
export API_KEY="mvk_your_api_key"
export BASE_URL="http://localhost:8080"
```

A Task runs in a Workspace; every registered repository has one. Pick its id:

```bash
WORKSPACE_ID=$(curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/workspaces" | jq -r '.[0].id')
```

Create the task with its first message, which starts the agent:

```bash
TASK_ID=$(curl -s -X POST -H "Authorization: Bearer $API_KEY" -H "Content-Type: application/json" \
  -d '{"workspaceId":"'"$WORKSPACE_ID"'","message":"Add a hello world endpoint"}' \
  "$BASE_URL/api/v1/tasks" | jq -r '.taskId')
```

Send a follow-up message, then read the conversation:

```bash
curl -s -X POST -H "Authorization: Bearer $API_KEY" -H "Content-Type: application/json" \
  -d '{"message":"Please also add tests"}' "$BASE_URL/api/v1/tasks/$TASK_ID/messages"

curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID/messages"
```

The `moat tasks` commands wrap the same calls.

## Errors

Every API error has the body `{"error": "<message>"}`.

| Code | Meaning |
|------|---------|
| 400 | Bad request (invalid parameters) |
| 401 | Missing, invalid, revoked or expired credential |
| 403 | Authenticated, but not allowed |
| 404 | Resource not found |
| 429 | Too many failed login attempts |

## Security practices

1. **Never commit API keys** to version control.
2. **Set expiration dates** with `expiresInDays` where you can.
3. **Use separate keys** for different purposes (dev, CI, production), and
   watch each key's `lastUsedAt`.
4. **Revoke unused keys**: `DELETE /api/v1/auth/api-keys/{keyId}`. A revoked
   key is invalid immediately and cannot be restored.
