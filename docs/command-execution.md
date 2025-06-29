# Command Execution Feature

This document describes how to use the command execution feature in the Moatless Swarm UI.

## Overview

The command execution feature allows you to define reusable commands with predefined prompts that can be executed on tasks. This is useful for common operations like committing changes, running tests, or deploying code.

## Implementation Details

### 1. Command Storage
Commands are stored in `data/commands.json` with the following structure:
```json
{
  "commands": {
    "commit": {
      "name": "commit",
      "description": "Commit the changes to the repository",
      "prompt": "commit the relevant changes"
    }
  }
}
```

### 2. API Endpoint
To execute a command on a task, send a POST request to `/api/tasks/{taskId}/messages` with:
```json
{
  "message": "This will be ignored if commandName is provided",
  "commandName": "commit",
  "resume": true,
  "model": "claude-3-5-sonnet-20241022"
}
```

### 3. How It Works
1. When a `commandName` is provided in the send message API call:
   - The TaskService looks up the command in the CommandService
   - If found, it uses the command's prompt instead of the provided message
   - The command name is stored in the TaskIteration for tracking
   
2. The agent receives the command prompt as the message to execute

3. The TaskIteration includes the `commandName` field to track which command was executed

### 4. Updated Types
- `TaskIteration` now includes an optional `commandName` field
- `SendMessageRequest` API schema includes an optional `commandName` parameter

## Example Usage

1. Create a command using the Commands API:
```bash
POST /api/commands
{
  "name": "test",
  "description": "Run the test suite",
  "prompt": "run the test suite and fix any failing tests"
}
```

2. Execute the command on a task:
```bash
POST /api/tasks/{taskId}/messages
{
  "commandName": "test",
  "resume": true
}
```

3. The agent will receive "run the test suite and fix any failing tests" as the message.

## Benefits
- Consistency: Ensures the same prompt is used for common operations
- Reusability: Commands can be reused across different tasks
- Tracking: The system tracks which command was used for each iteration
- Flexibility: Commands can be updated without changing client code