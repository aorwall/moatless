---
name: e2e-testing
description: Execute end-to-end UI tests for the Moatless Vibe application. Tests repository configuration, task creation, agent execution, preview functionality, and screenshot verification. Use when validating application workflows, testing new features, or running regression tests. Supports both automated API-based testing and UI verification.
---

# E2E Testing Skill

Execute comprehensive end-to-end tests of the Moatless Vibe application using the MCP preview tools and API endpoints.

## Overview

This skill provides a framework for testing Moatless Vibe UI workflows end-to-end. It uses:
- **API calls** for reliable task creation and status monitoring (avoids UI automation issues)
- **MCP preview tools** for navigation and screenshot verification
- **Modular test cases** for different features (repository setup, task execution, etc.)

## Prerequisites

Before running tests:
- Moatless Vibe instance must be running
- Preview server accessible (typically `http://localhost:PORT`)
- MCP tools available: `mcp__moatless-tools__*`

## Available Test Cases

Test cases are defined in the `test-cases/` directory:

1. **Repository and Task Creation** (`test-cases/repo-and-task.md`)
   - Verify repository configuration
   - Create tasks via API
   - Monitor agent execution
   - Verify screenshots and preview

More test cases can be added as separate files in `test-cases/`.

## Test Execution Workflow

When running tests, follow this general pattern:

### 1. Setup and Verification
- Navigate to dashboard (`/`)
- Verify application is accessible
- Check prerequisites (repositories, configuration)

### 2. Execute Test Steps
- Use API calls for reliable data operations
- Use MCP tools for UI verification
- Document each step's result

### 3. Verification
- Check expected outcomes (screenshots, status changes, etc.)
- Use appropriate verification methods:
  - API: Data integrity, status changes
  - Screenshots: Visual confirmation of UI elements
  - Logs: Error detection, timing

### 4. Report Results
- Provide structured test report with pass/fail status
- Include timing measurements
- Document any issues found
- Suggest troubleshooting steps if failures occur

## General Testing Best Practices

### API-Based Operations (Preferred)

**Always use API for task creation** to avoid duplicate tasks:

```bash
# Create task via API
curl -X POST http://localhost:PORT/api/v1/tasks \
  -H "Content-Type: application/json" \
  -d '{
    "agentType": "claude-code",
    "repositoryId": "REPO_ID",
    "message": "Task description here",
    "model": "claude-sonnet-4-5",
    "agentMode": "edit"
  }'
```

**Why**: UI automation with `typeText` can trigger multiple form submissions when handling multi-line text, causing duplicate tasks. The production code has proper safeguards, but API usage is more reliable for testing.

### UI Verification (Screenshots)

Use MCP tools for visual verification:

```javascript
// Navigate to page
mcp__moatless-tools__navigatePreview({ path: "/tasks/TASK_ID" })

// Take screenshot
mcp__moatless-tools__takePreviewScreenshot()

// Verify elements via accessibility snapshot
mcp__moatless-tools__takePreviewSnapshot()
```

**Note**: Direct iframe interaction (clicking, typing) may not work due to security restrictions. Use screenshots and accessibility snapshots for verification.

### Status Monitoring

Poll APIs for status changes:

```bash
# Check task status
curl http://localhost:PORT/api/v1/tasks/TASK_ID | jq '{status, numTurns}'

# Monitor until completion
while [ "$(curl -s http://localhost:PORT/api/v1/tasks/TASK_ID | jq -r '.status')" = "running" ]; do
  echo "Agent still running..."
  sleep 15
done
```

## Running a Test

To execute a specific test case:

1. **Identify the test case** you want to run (e.g., repo-and-task)
2. **Read the test case file** for specific instructions
3. **Execute the steps** using the appropriate tools (API, MCP tools)
4. **Verify results** against success criteria
5. **Generate test report** with findings

## Test Report Format

Each test should produce a structured report:

```markdown
## Test Report: [Test Name]

**Date**: [timestamp]
**Duration**: [time]
**Result**: PASS/FAIL

### Test Summary
- [Brief overview of what was tested]
- [Overall outcome]

### Detailed Results
| Step | Status | Notes |
|------|--------|-------|
| Step 1 | ✅ PASS | [details] |
| Step 2 | ❌ FAIL | [error message] |

### Timing Measurements
- Task creation: [time]
- Agent execution: [time]
- Total: [time]

### Issues Found
- [Issue 1 description]
- [Issue 2 description]

### Recommendations
- [Suggestion 1]
- [Suggestion 2]
```

## Common Verification Patterns

### Verify Repository Configuration

```bash
curl http://localhost:PORT/api/v1/repositories | \
  jq '.repositories[] | select(.name == "REPO_NAME")'
```

Expected fields:
- `codeExecution.setupCommands`: Array of setup commands
- `previewConfig.serverStartCommands`: Array of server start commands
- `previewConfig.takeScreenshots`: `true`
- `previewConfig.autoReload`: `true`

### Verify Task Creation

```bash
# Check no duplicates created within last minute
curl http://localhost:PORT/api/v1/tasks?limit=20 | \
  jq '.tasks[0:5] | group_by(.name) | map(select(length > 1))'
```

### Verify Agent Completion

```bash
curl http://localhost:PORT/api/v1/tasks/TASK_ID | \
  jq '{
    status,
    numTurns,
    hasScreenshot: (.iterations[0].screenshot_file_id != null),
    completionTime: .iterations[0].end_time
  }'
```

## Error Handling

If a test fails:

1. **Capture diagnostic information**:
   - API response bodies
   - Screenshot of current UI state
   - Console logs if available
   - Server logs via `mcp__moatless-tools__getPreviewServerLogs`

2. **Check common issues**:
   - JSON parse errors → Repository configuration format
   - Task not starting → Repository ID or API endpoint
   - No screenshot → `takeScreenshots` setting
   - Duplicate tasks → UI vs API creation method

3. **Provide actionable guidance**:
   - Reference troubleshooting documentation
   - Suggest API commands to diagnose
   - Recommend manual verification steps

## Resources

- **Test Cases**: `test-cases/` - Individual test scenarios
- **Test Data**: `resources/test-data.md` - Sample prompts and configurations
- **Reference**: `resources/api-reference.md` - API endpoint documentation

For detailed test case instructions, see the files in `test-cases/` directory.
