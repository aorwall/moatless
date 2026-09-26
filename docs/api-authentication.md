# API Authentication Guide

This guide explains how to authenticate with the Moatless Vibe API.

There are two credentials: a **session cookie** (browsers and the CLI login
flow) and an **API key** (programmatic access). Every API route requires one or
the other — there is no unauthenticated mode.

## Password login

When the server runs local username/password auth (`AUTH_PROVIDER` unset or
`local`), `POST /api/v1/auth/password/login` exchanges credentials for a session
cookie:

```bash
curl -i -X POST -H "Content-Type: application/json" \
  -d '{"username":"admin","password":"your-password"}' \
  "$BASE_URL/api/v1/auth/password/login"
# → 200 + Set-Cookie: moatless_session=...
```

`GET /api/v1/auth/mode` reports which flow a deployment runs (`password` or
`oauth`) and is public, so a login page can call it before authenticating.

The bootstrap `admin` account is seeded at startup from `ADMIN_PASSWORD`. When
that is unset, a random password is generated on first boot and logged **once**
at WARN. Change it under Settings → Password, or reset another user's with
`PUT /api/v1/users/{login}/password` as an admin.

On OAuth deployments the password endpoints answer 404 and `/api/v1/auth/login`
redirects to the configured provider instead.

## Overview

API keys provide programmatic access to the Moatless Vibe API. They can be used for:
- CI/CD pipelines
- Automated testing
- Third-party integrations
- Custom tooling

## Creating an API Key

1. Open the Settings dialog in the web interface
2. Navigate to the **API Keys** section
3. Click **Create API Key**
4. Enter a descriptive name (e.g., "CI/CD Pipeline", "E2E Tests")
5. Optionally set an expiration period
6. Click **Create Key**

**Important**: The full API key is only shown once. Copy and store it securely.

API keys have the format: `mvk_<random_string>`

## Using the API Key

Include the API key in the `Authorization` header with the `Bearer` scheme:

```bash
Authorization: Bearer mvk_your_api_key_here
```

> **Shell Compatibility Note**: The curl examples in this guide work best when:
> - Using single-line commands (most reliable for copy-paste)
> - Variables are exported before use: `export API_KEY="mvk_..."`
> - If using multi-line commands with `\`, ensure no trailing whitespace after the backslash

### Example: List Repositories

```bash
curl -H "Authorization: Bearer mvk_your_api_key" "https://moatless.xyz/api/v1/repositories"
```

Response:
```json
{
  "repositories": [
    {
      "id": "3c9b49b9-e9a9-459b-905f-55617b677ffd",
      "name": "my-project",
      "kind": "remote",
      "visibility": "private",
      ...
    }
  ]
}
```

## API Endpoints

### Repositories

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/repositories` | List all repositories |
| POST | `/api/v1/repositories` | Create a repository |
| GET | `/api/v1/repositories/{id}` | Get repository details |
| PUT | `/api/v1/repositories/{id}` | Update a repository |
| DELETE | `/api/v1/repositories/{id}` | Delete a repository |
| GET | `/api/v1/repositories/{id}/branches` | List branches |

### Tasks

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/tasks` | List all tasks |
| POST | `/api/v1/tasks` | Create a new task |
| GET | `/api/v1/tasks/{taskId}` | Get task details |
| DELETE | `/api/v1/tasks/{taskId}` | Delete a task |
| POST | `/api/v1/tasks/{taskId}/start` | Start a task |
| POST | `/api/v1/tasks/{taskId}/stop` | Stop a task |
| POST | `/api/v1/tasks/{taskId}/messages` | Send a message to the agent |
| GET | `/api/v1/tasks/{taskId}/messages` | Get task messages |

### Workspace

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/tasks/{taskId}/workspace-status` | Get workspace status |
| GET | `/api/v1/tasks/{taskId}/files` | List files |
| GET | `/api/v1/tasks/{taskId}/files/{path}` | Get file content |
| PUT | `/api/v1/tasks/{taskId}/files/{path}` | Update file content |
| GET | `/api/v1/tasks/{taskId}/git-diff` | Get git diff |

### API Keys Management

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/auth/api-keys` | List your API keys |
| POST | `/api/v1/auth/api-keys` | Create a new API key |
| DELETE | `/api/v1/auth/api-keys/{keyId}` | Revoke an API key |

## Common Workflows

### Create and Run a Task

First, set up your environment variables:

```bash
export API_KEY="mvk_your_api_key"
export BASE_URL="https://moatless.xyz"
```

**Step 1: Get repository ID**

```bash
REPO_ID=$(curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/repositories" | jq -r '.repositories[0].id')
echo "Repository ID: $REPO_ID"
```

**Step 2: Create a task**

Option A - Single-line (most reliable for copy-paste):
```bash
TASK_RESPONSE=$(curl -s -X POST -H "Authorization: Bearer $API_KEY" -H "Content-Type: application/json" -d '{"agentProfile":"claude-code","repositoryId":"'"$REPO_ID"'","message":"Add a hello world endpoint","start":true}' "$BASE_URL/api/v1/tasks")
```

Option B - Using a script file (better for complex requests):
```bash
# Create a file called create-task.sh
cat > create-task.sh << 'SCRIPT'
#!/bin/bash
curl -s -X POST \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d "$(cat <<EOF
{
  "agentProfile": "claude-code",
  "repositoryId": "$REPO_ID",
  "message": "Add a hello world endpoint",
  "start": true
}
EOF
)" "$BASE_URL/api/v1/tasks"
SCRIPT
chmod +x create-task.sh
TASK_RESPONSE=$(./create-task.sh)
```

Extract the task ID:
```bash
TASK_ID=$(echo $TASK_RESPONSE | jq -r '.taskId')
echo "Created task: $TASK_ID"
```

**Step 3: Check task status**

```bash
curl -s -H "Authorization: Bearer $API_KEY" "$BASE_URL/api/v1/tasks/$TASK_ID" | jq '{status, numTurns}'
```

**Step 4: Send a follow-up message**

```bash
curl -s -X POST -H "Authorization: Bearer $API_KEY" -H "Content-Type: application/json" -d '{"message":"Please also add tests"}' "$BASE_URL/api/v1/tasks/$TASK_ID/messages"
```

### Poll for Task Completion

Set up variables (if not already set):
```bash
export API_KEY="mvk_your_api_key"
export TASK_ID="your-task-id"
```

**Option A - Using `watch` (simple, interactive):**
```bash
watch -n 5 'curl -s -H "Authorization: Bearer $API_KEY" "https://moatless.xyz/api/v1/tasks/$TASK_ID" | jq "{status, numTurns}"'
```

**Option B - Bash loop (for scripts):**
```bash
while true; do
  STATUS=$(curl -s -H "Authorization: Bearer $API_KEY" "https://moatless.xyz/api/v1/tasks/$TASK_ID" | jq -r '.status')
  echo "Status: $STATUS"
  if [[ "$STATUS" == "completed" || "$STATUS" == "failed" ]]; then
    break
  fi
  sleep 5
done
```

### Get Task Results

```bash
# Get the git diff (changes made by the agent)
curl -s -H "Authorization: Bearer $API_KEY" "https://moatless.xyz/api/v1/tasks/$TASK_ID/git-diff"

# Get all messages from the conversation
curl -s -H "Authorization: Bearer $API_KEY" "https://moatless.xyz/api/v1/tasks/$TASK_ID/messages"
```

## Error Handling

### Authentication Errors

```json
{
  "error": {
    "code": "UNAUTHORIZED",
    "message": "Invalid or expired API key"
  }
}
```

Possible causes:
- Invalid API key format
- API key has been revoked
- API key has expired

### Common HTTP Status Codes

| Code | Meaning |
|------|---------|
| 200 | Success |
| 400 | Bad request (invalid parameters) |
| 401 | Unauthorized (invalid/missing API key) |
| 403 | Forbidden (insufficient permissions) |
| 404 | Resource not found |
| 500 | Internal server error |

## Security Best Practices

1. **Never commit API keys** to version control
2. **Use environment variables** to store API keys
3. **Set expiration dates** for API keys when possible
4. **Revoke unused keys** promptly
5. **Use separate keys** for different purposes (dev, CI, production)
6. **Monitor usage** through the `lastUsedAt` field

## Environment Variables

For scripts and CI/CD, use environment variables:

```bash
export MOATLESS_API_KEY="mvk_your_api_key"

# Then in your scripts:
curl -H "Authorization: Bearer $MOATLESS_API_KEY" ...
```

## Rate Limits

API keys are subject to the same rate limits as session-based authentication. Contact support if you need higher limits for automation use cases.

## Revoking API Keys

To revoke an API key:

1. **Via Web Interface**: Go to Settings > API Keys, click the trash icon
2. **Via API**:
   ```bash
   # Replace {keyId} with the actual key ID (e.g., "abc123")
   curl -X DELETE -H "Authorization: Bearer $API_KEY" "https://moatless.xyz/api/v1/auth/api-keys/{keyId}"
   ```

Revoked keys are immediately invalidated and cannot be restored.

## Troubleshooting

### "Authentication required" Error

```json
{"error":{"code":"HTTP_ERROR","message":"Authentication required"}}
```

**Causes:**
- API key not set or empty
- Variable not exported (use `export API_KEY=...`)
- Typo in the Authorization header

**Solution:** Verify your API key is set correctly:
```bash
echo $API_KEY  # Should print your key
```

### "blank argument where content is expected" Error

**Cause:** Multi-line curl commands with backslash continuations can break if there's whitespace after `\` or when copying from documentation.

**Solution:** Use single-line curl commands instead:
```bash
# Instead of multi-line:
curl -H "Header: value" \
  "https://example.com"

# Use single-line:
curl -H "Header: value" "https://example.com"
```

### Variable Not Expanding

If `$API_KEY` or `$TASK_ID` appears literally in the URL instead of being replaced:

**Causes:**
- Using single quotes around the entire command
- Variable not exported

**Solution:**
```bash
# Make sure to export variables
export API_KEY="mvk_your_key"

# Use double quotes for strings containing variables
curl -H "Authorization: Bearer $API_KEY" "https://..."
```
