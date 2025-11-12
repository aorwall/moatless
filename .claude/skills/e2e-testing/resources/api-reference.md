# API Reference for E2E Testing

Quick reference for API endpoints used in testing.

## Base URL

Replace `PORT` with the actual port (typically `50843` or check server logs).

```
http://localhost:PORT/api/v1
```

## Repository Endpoints

### List Repositories
```bash
GET /api/v1/repositories

# Example
curl http://localhost:PORT/api/v1/repositories | jq '.'

# Filter by name
curl http://localhost:PORT/api/v1/repositories | \
  jq '.repositories[] | select(.name == "hello-world-ui")'
```

**Response**: `{ "repositories": [...] }`

### Get Repository
```bash
GET /api/v1/repositories/{id}

# Example
curl http://localhost:PORT/api/v1/repositories/REPO-ID | jq '.'
```

## Task Endpoints

### Create Task
```bash
POST /api/v1/tasks
Content-Type: application/json

{
  "agentType": "claude-code",
  "repositoryId": "repo-id",
  "message": "Task description (optional)",
  "model": "claude-sonnet-4-5",
  "agentMode": "edit",
  "originalBranch": "main",
  "fileIds": [],
  "contexts": []
}

# Example
curl -X POST http://localhost:PORT/api/v1/tasks \
  -H "Content-Type: application/json" \
  -d '{
    "agentType": "claude-code",
    "repositoryId": "REPO-ID",
    "message": "Create a todo app...",
    "model": "claude-sonnet-4-5",
    "agentMode": "edit"
  }'
```

**Response**: `{ "taskId": "...", "status": "..." }`

**Notes**:
- If `message` provided, agent starts immediately
- If `message` omitted, task waits for first message
- `agentMode`: `"edit"` (makes changes) or `"plan"` (creates plans)

### List Tasks
```bash
GET /api/v1/tasks?limit=50&offset=0

# Example
curl http://localhost:PORT/api/v1/tasks?limit=20 | jq '.tasks'

# Get recent tasks with specific info
curl http://localhost:PORT/api/v1/tasks?limit=10 | \
  jq '.tasks[] | {id: .id, name: .name, status: .status, created: .createdAt}'
```

**Response**: `{ "tasks": [...], "total": N }`

### Get Task Details
```bash
GET /api/v1/tasks/{taskId}

# Example
curl http://localhost:PORT/api/v1/tasks/TASK-ID | jq '.'

# Get specific fields
curl http://localhost:PORT/api/v1/tasks/TASK-ID | \
  jq '{status, numTurns, iterations}'
```

**Response**: Full task object with iterations, messages, status

### Get Task Server Status
```bash
GET /api/v1/tasks/{taskId}/server/status

# Example
curl http://localhost:PORT/api/v1/tasks/TASK-ID/server/status | jq '.'
```

**Response**: Server status, port, and health information

### Send Message to Task
```bash
POST /api/v1/tasks/{taskId}/messages
Content-Type: application/json

{
  "message": "Follow-up message",
  "resume": true,
  "agentMode": "edit",
  "model": "claude-sonnet-4-5"
}

# Example
curl -X POST http://localhost:PORT/api/v1/tasks/TASK-ID/messages \
  -H "Content-Type: application/json" \
  -d '{
    "message": "Change the background color to purple",
    "resume": true
  }'
```

**Notes**:
- `resume: true` continues from previous state
- Task must be in `open` status (not `running`)

## Common Query Patterns

### Check for Duplicate Tasks
```bash
# Get tasks created in last 5 minutes with same description
curl http://localhost:PORT/api/v1/tasks?limit=50 | \
  jq '.tasks |
    map(select(.createdAt > (now - 300 | strftime("%Y-%m-%dT%H:%M:%S.000Z")))) |
    group_by(.name) |
    map(select(length > 1))'
```

### Monitor Task Until Completion
```bash
TASK_ID="your-task-id"

while true; do
  STATUS=$(curl -s http://localhost:PORT/api/v1/tasks/$TASK_ID | jq -r '.status')
  echo "$(date +%T) - Status: $STATUS"

  if [ "$STATUS" = "open" ] || [ "$STATUS" = "error" ]; then
    break
  fi

  sleep 15
done
```

### Verify Agent Completed Successfully
```bash
curl http://localhost:PORT/api/v1/tasks/TASK-ID | \
  jq '{
    status,
    numTurns,
    completed: (.status == "open" and .numTurns > 0),
    hasScreenshot: (.iterations[0].screenshot_file_id != null),
    duration: (
      (.iterations[0].end_time | fromdateiso8601) -
      (.iterations[0].start_time | fromdateiso8601)
    )
  }'
```

### Get Repository ID by Name
```bash
REPO_ID=$(curl -s http://localhost:PORT/api/v1/repositories | \
  jq -r '.repositories[] | select(.name == "hello-world-ui") | .id')

echo "Repository ID: $REPO_ID"
```

## Task Status Values

- `open`: Ready for messages, agent not running
- `running`: Agent actively processing
- `error`: Task encountered an error
- `merged`: Changes merged to main branch
- `closed`: Task closed, workspace cleaned up

## Response Field Reference

### Task Object (minimal)
```json
{
  "id": "task-uuid",
  "name": "task-name",
  "status": "open|running|error|merged|closed",
  "numTurns": 0,
  "createdAt": "2025-11-12T13:53:23.571Z",
  "iterations": [...]
}
```

### Iteration Object (minimal)
```json
{
  "iteration_number": 1,
  "status": "completed|running|error",
  "start_time": "2025-11-12T13:53:25.000Z",
  "end_time": "2025-11-12T13:55:32.000Z",
  "initialMessage": "Create a todo app...",
  "screenshot_file_id": "screenshot-uuid",
  "screenshot_path": "/path/to/screenshot.png"
}
```

## Error Responses

### JSON Parse Error
```json
{
  "error": "HTTP_ERROR",
  "message": "JSON Parse error: Unexpected identifier \"bun\""
}
```

**Cause**: Repository commands stored as strings instead of arrays

**Fix**: Use UI to re-save repository configuration

### Repository Not Found
```json
{
  "error": "NOT_FOUND",
  "message": "Repository not found"
}
```

**Cause**: Invalid repository ID or repository deleted

### Task Not Found
```json
{
  "error": "NOT_FOUND",
  "message": "Task not found"
}
```

**Cause**: Invalid task ID or task deleted

## Rate Limiting

No explicit rate limits for local development, but avoid rapid polling:
- Use 10-15 second intervals for status checks
- Batch API calls when possible
- Use event streams if available for real-time updates
