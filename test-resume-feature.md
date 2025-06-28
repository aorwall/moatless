# Test Resume Feature

## How to test the new resume parameter

1. Send a message to start a new task with default behavior (resume=true):
```json
POST /api/tasks/{taskId}/messages
{
  "message": "First message"
}
```

2. Send a second message that will resume from the previous session:
```json
POST /api/tasks/{taskId}/messages
{
  "message": "Second message"
}
```

3. Send a message that opts out of resuming:
```json
POST /api/tasks/{taskId}/messages
{
  "message": "New conversation without resume",
  "resume": false
}
```

## What to expect

- When `resume` is not specified or set to `true`, the agent will use the previous session ID to continue the conversation
- When `resume` is set to `false`, the agent will start a fresh conversation without any context from previous messages

## Implementation details

1. **API Schema Update**: Added optional `resume` parameter to `SendMessageRequestSchema` with default value of `true`
2. **Route Handler**: Updated to extract and pass the `resume` parameter to the service
3. **Task Service**: Modified `sendMessage` method to conditionally set `resumeSessionId` based on the `resume` parameter
4. **Agent Service**: Already handles the `resumeSessionId` parameter correctly