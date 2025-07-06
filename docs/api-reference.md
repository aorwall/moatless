# API Reference

## Overview

The Moatless Swarm UI API is a RESTful API that provides endpoints for managing tasks, repositories, and AI agent operations. All endpoints are prefixed with `/api/v1`.

## Base URL

```
http://localhost:8001/api/v1
```

## Authentication

Currently, the API does not require authentication. This will be added in future versions.

## Error Responses

All errors follow a consistent format:

```json
{
  "error": {
    "message": "Error description",
    "code": "ERROR_CODE",
    "statusCode": 400
  }
}
```

Common HTTP status codes:
- `200` - Success
- `201` - Created
- `204` - No Content
- `400` - Bad Request
- `404` - Not Found
- `500` - Internal Server Error

## Endpoints

### Tasks

#### Create Task
```http
POST /tasks
Content-Type: application/json

{
  "title": "Task title",
  "description": "Task description",
  "repositoryId": "repo-123",
  "model": "claude-3-5-sonnet-20241022",
  "filePaths": ["/src/index.ts"]
}
```

Response:
```json
{
  "taskId": "task-123",
  "status": "open"
}
```

#### Create Vibe Task
```http
POST /tasks/vibe
Content-Type: application/json

{
  "title": "Task title",
  "template": "bug-fix",
  "repositoryId": "repo-123",
  "model": "claude-3-5-sonnet-20241022",
  "filePaths": ["/src/index.ts"]
}
```

#### Get Task
```http
GET /tasks/:taskId
```

Response:
```json
{
  "id": "task-123",
  "title": "Task title",
  "description": "Task description",
  "status": "running",
  "repositoryId": "repo-123",
  "iterations": [...],
  "createdAt": "2024-01-01T00:00:00Z",
  "updatedAt": "2024-01-01T00:00:00Z"
}
```

#### List Tasks
```http
GET /tasks?limit=10&offset=0&status=open
```

Query parameters:
- `limit` - Number of tasks to return (default: 20)
- `offset` - Pagination offset (default: 0)
- `status` - Filter by status (open, running, completed, failed)

#### Send Message
```http
POST /tasks/:taskId/messages
Content-Type: application/json

{
  "message": "Please fix the bug in the login function",
  "resume": false,
  "model": "claude-3-5-sonnet-20241022",
  "fileIds": [],
  "filePaths": [],
  "commandName": null
}
```

#### Get Task Events
```http
GET /tasks/:taskId/events?type=message
```

Query parameters:
- `type` - Filter by event type

#### Close Task
```http
POST /tasks/:taskId/close
```

#### Stop Agent
```http
POST /tasks/:taskId/stop
```

#### Delete Task
```http
DELETE /tasks/:taskId
```

#### Merge Task Changes
```http
POST /tasks/:taskId/merge
Content-Type: application/json

{
  "targetBranch": "main",
  "commitMessage": "Fix: resolved login bug"
}
```

### Workspace Operations

#### Get Workspace Status
```http
GET /tasks/:taskId/workspace-status
```

Response:
```json
{
  "status": "IDLE",
  "canExecute": true,
  "hasChanges": true,
  "gitSyncStatus": {
    "currentBranch": "task-123",
    "baseBranch": "main",
    "commitsAhead": 2,
    "commitsBehind": 0,
    "hasUncommittedChanges": true
  }
}
```

#### List Files
```http
GET /tasks/:taskId/files?path=/src&recursive=true
```

Query parameters:
- `path` - Directory path (default: "/")
- `recursive` - Include subdirectories (default: false)

#### Read File
```http
GET /tasks/:taskId/files/src/index.ts
```

#### Write File
```http
PUT /tasks/:taskId/files/src/index.ts
Content-Type: application/json

{
  "content": "console.log('Hello World');"
}
```

#### Search Files
```http
POST /tasks/:taskId/search-files
Content-Type: application/json

{
  "query": "login",
  "limit": 50
}
```

#### Get Git Diff
```http
GET /tasks/:taskId/git-diff
```

#### Execute Command
```http
POST /tasks/:taskId/execute
Content-Type: application/json

{
  "command": "npm test",
  "timeout": 30000
}
```

### Server Management

#### Start Server
```http
POST /tasks/:taskId/server/start
```

#### Stop Server
```http
POST /tasks/:taskId/server/stop
```

#### Restart Server
```http
POST /tasks/:taskId/server/restart
```

#### Get Server Status
```http
GET /tasks/:taskId/server/status
```

#### Get Server Logs
```http
GET /tasks/:taskId/server/logs?lines=100
```

### Repositories

#### Create Repository
```http
POST /repositories
Content-Type: application/json

{
  "name": "My Project",
  "path": "/path/to/repo",
  "useGitWorktrees": true,
  "originalBranch": "main",
  "codeExecution": {
    "type": "local",
    "setupCommands": "npm install",
    "serverCommand": "npm run dev -- --port {{port}}"
  }
}
```

#### List Repositories
```http
GET /repositories
```

#### Get Repository
```http
GET /repositories/:repositoryId
```

#### Update Repository
```http
PUT /repositories/:repositoryId
Content-Type: application/json

{
  "name": "Updated Name",
  "codeExecution": {
    "serverCommand": "npm start"
  }
}
```

#### Delete Repository
```http
DELETE /repositories/:repositoryId
```

#### Run Setup Script
```http
POST /repositories/:repositoryId/run-setup
```

#### Search Files in Repository
```http
POST /repositories/:repositoryId/search-files
Content-Type: application/json

{
  "query": "TODO",
  "limit": 100
}
```

#### Get Repository Branches
```http
GET /repositories/:repositoryId/branches
```

### Events

#### List Events
```http
GET /events?taskId=task-123&limit=100&offset=0&type=message
```

Query parameters:
- `taskId` - Filter by task ID
- `limit` - Number of events to return
- `offset` - Pagination offset
- `type` - Filter by event type

#### Event Stream (SSE)
```http
GET /events/stream
Accept: text/event-stream
```

Query parameters:
- `types` - Comma-separated event types to filter
- `activeTasksOnly` - Only stream from active tasks
- `lastEventId` - Resume from specific event

### Commands

#### List Commands
```http
GET /commands
```

Response:
```json
{
  "commands": [
    {
      "name": "test",
      "description": "Run tests",
      "command": "npm test"
    }
  ]
}
```

### Uploads

#### Upload File
```http
POST /uploads
Content-Type: multipart/form-data

file: [binary data]
```

Response:
```json
{
  "fileId": "file-123",
  "filename": "document.pdf",
  "size": 1024
}
```

## Event Types

Events are used for real-time updates via Server-Sent Events:

- `input` - User input received
- `message` - AI agent message
- `start` - Task started
- `stop` - Task stopped
- `result` - Task completed
- `error` - Error occurred
- `workspace` - Workspace status changed
- `server` - Server status changed
- `tool` - Tool execution event
- `completion` - Code completion event

## Rate Limiting

Currently, there are no rate limits. This will be added in future versions.

## Webhooks

Webhook support is planned for future versions.

## SDK Support

TypeScript types are available in the `@moatless/shared` package:

```typescript
import type { Task, Repository, Event } from '@moatless/shared/types'
```