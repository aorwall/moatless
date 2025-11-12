# Test Case: Repository and Task Creation

Test the complete workflow of repository configuration verification and task creation with agent execution.

## Test Scope

This test validates:
- Repository configuration (setup commands, server commands, screenshots)
- Task creation via API (avoiding duplicate task issues)
- Agent execution and completion timing
- Screenshot capture and verification
- Preview server functionality

## Reference Documentation

Detailed manual testing guide: `docs/testing-repo-and-task-creation.md`

## Test Steps

### 1. Verify Repository Configuration

**Repository**: `hello-world-ui` from `https://github.com/aorwall/hello-world-ui.git`

1. Navigate to dashboard:
   ```javascript
   mcp__moatless-tools__navigatePreview({ path: "/" })
   mcp__moatless-tools__takePreviewSnapshot()
   ```

2. Check repository exists and is configured:
   ```bash
   curl http://localhost:PORT/api/v1/repositories | \
     jq '.repositories[] | select(.name == "hello-world-ui")'
   ```

3. Verify configuration:
   - ✅ `codeExecution.setupCommands`: `["bun install"]`
   - ✅ `previewConfig.serverStartCommands`: `["bun run dev"]`
   - ✅ `previewConfig.takeScreenshots`: `true`
   - ✅ `previewConfig.autoReload`: `true`
   - ✅ `previewConfig.enableInspector`: `true`

**If repository is missing or misconfigured**: Document the issue and refer to manual setup in the testing guide.

### 2. Create Task via API

**Critical**: Use API, not UI automation, to avoid duplicate tasks.

1. Get repository ID:
   ```bash
   REPO_ID=$(curl -s http://localhost:PORT/api/v1/repositories | \
     jq -r '.repositories[] | select(.name == "hello-world-ui") | .id')
   ```

2. Prepare task description (use the standard todo app prompt from `resources/test-data.md`)

3. Create task:
   ```bash
   TASK_RESPONSE=$(curl -s -X POST http://localhost:PORT/api/v1/tasks \
     -H "Content-Type: application/json" \
     -d "{
       \"agentType\": \"claude-code\",
       \"repositoryId\": \"$REPO_ID\",
       \"message\": \"Create a todo app with the following features: Add new todos with a text input and submit button, Display list of todos with checkboxes, Mark todos as complete/incomplete by clicking checkbox, Delete todos with a delete button, Show count of active todos, Filter todos by: All, Active, Completed, Use Tailwind CSS for styling, Use TypeScript for type safety\",
       \"model\": \"claude-sonnet-4-5\",
       \"agentMode\": \"edit\"
     }")

   TASK_ID=$(echo $TASK_RESPONSE | jq -r '.taskId')
   echo "Created task: $TASK_ID"
   ```

4. Record task creation time

### 3. Verify Single Task Created

**Critical Check**: Ensure no duplicate tasks

```bash
# Check recent tasks for duplicates
curl http://localhost:PORT/api/v1/tasks?limit=20 | \
  jq '.tasks[0:10] | map({id: .id, name: .name, created: .createdAt})'
```

**Expected**: Only one task with the todo app description, created at the current timestamp.

**If duplicates found**: Document as FAIL and note the creation method used.

### 4. Monitor Agent Execution

1. Navigate to task page:
   ```javascript
   mcp__moatless-tools__navigatePreview({ path: `/tasks/${TASK_ID}` })
   ```

2. Record start time and monitor status:
   ```bash
   START_TIME=$(date +%s)

   while true; do
     STATUS=$(curl -s http://localhost:PORT/api/v1/tasks/$TASK_ID | jq -r '.status')
     echo "$(date +%T) - Task status: $STATUS"

     if [ "$STATUS" = "open" ]; then
       END_TIME=$(date +%s)
       DURATION=$((END_TIME - START_TIME))
       echo "Agent completed in $DURATION seconds"
       break
     fi

     if [ "$STATUS" = "error" ]; then
       echo "Agent failed!"
       break
     fi

     # Timeout after 5 minutes
     ELAPSED=$(($(date +%s) - START_TIME))
     if [ $ELAPSED -gt 300 ]; then
       echo "Timeout: Agent did not complete in 5 minutes"
       break
     fi

     sleep 15
   done
   ```

3. Verify agent activity:
   ```bash
   curl http://localhost:PORT/api/v1/tasks/$TASK_ID | \
     jq '{
       status,
       numTurns,
       hasIterations: (.iterations | length > 0),
       initialMessage: .iterations[0].initialMessage
     }'
   ```

**Expected**:
- Status: `open` (after completion)
- numTurns: > 0
- Iterations: At least 1
- Duration: 1-3 minutes typically

### 5. Verify Screenshot Captured

1. Check task details for screenshot:
   ```bash
   curl http://localhost:PORT/api/v1/tasks/$TASK_ID | \
     jq '.iterations[0] | {
       screenshot_file_id,
       screenshot_path,
       thumbnail_path
     }'
   ```

2. Navigate to dashboard and verify screenshot visible:
   ```javascript
   mcp__moatless-tools__navigatePreview({ path: "/" })
   mcp__moatless-tools__takePreviewSnapshot()
   ```

3. Locate task in list and verify screenshot preview is shown

4. Take screenshot of the task page preview:
   ```javascript
   mcp__moatless-tools__navigatePreview({ path: `/tasks/${TASK_ID}` })
   mcp__moatless-tools__takePreviewScreenshot()
   ```

5. Screenshot Verification Checklist:
   - ✅ Todo app interface visible
   - ✅ Input field with placeholder text
   - ✅ "Add" button visible
   - ✅ Filter buttons (All, Active, Completed) present
   - ✅ Todo count display (e.g., "0 items left")
   - ✅ Tailwind CSS styling (gradient background, modern design)

### 6. Verify Preview Server

```bash
curl http://localhost:PORT/api/v1/tasks/$TASK_ID/server/status | \
  jq '{status, port}'
```

**Expected**: Server running with assigned port, no errors

## Success Criteria

All of the following must pass:

- ✅ Repository configuration verified
- ✅ Single task created (no duplicates)
- ✅ Task created via API without errors
- ✅ Agent started processing immediately
- ✅ Agent completed within 1-3 minutes (acceptable up to 5 minutes)
- ✅ Task status changed from "running" to "open"
- ✅ Preview server started automatically
- ✅ Screenshot captured showing todo app
- ✅ Screenshot shows all required UI elements
- ✅ Task card displays screenshot preview on dashboard

## Timing Benchmarks

- **Task creation**: < 1 second
- **Agent execution**: 1-3 minutes (typical), up to 5 minutes (acceptable)
- **Screenshot capture**: Available within 10 seconds of completion
- **Total test time**: ~2-4 minutes

## Common Failures and Troubleshooting

### Repository Not Found
- **Cause**: Repository not created or wrong name
- **Fix**: Verify repository exists via API or create manually
- **Reference**: `docs/testing-repo-and-task-creation.md` Step 1

### Duplicate Tasks Created
- **Cause**: Used UI automation instead of API
- **Fix**: Use API for task creation as shown in Step 2
- **Note**: This is a testing methodology issue, not an app bug

### Agent Timeout
- **Cause**: Complex task or performance issue
- **Action**: Check agent logs, verify task is actually stuck
- **Threshold**: 5 minutes is hard timeout

### No Screenshot
- **Cause**: `takeScreenshots` not enabled in repository config
- **Fix**: Update repository configuration
- **Verify**: Check `previewConfig.takeScreenshots: true`

### JSON Parse Error
- **Cause**: Repository commands not in array format
- **Fix**: Use UI to re-save repository configuration
- **Reference**: `docs/testing-repo-and-task-creation.md` Troubleshooting

## Test Report Template

```markdown
## Test Report: Repository and Task Creation

**Date**: [timestamp]
**Duration**: [X minutes]
**Result**: PASS/FAIL

### Summary
[Brief overview]

### Results
| Step | Status | Duration | Notes |
|------|--------|----------|-------|
| Repository verification | ✅ PASS | 2s | Config correct |
| Task creation | ✅ PASS | 0.5s | ID: abc-123 |
| Duplicate check | ✅ PASS | 1s | No duplicates |
| Agent execution | ✅ PASS | 127s | Completed successfully |
| Screenshot capture | ✅ PASS | 3s | All elements visible |
| Preview server | ✅ PASS | 1s | Running on port 54321 |

### Timing
- Task creation: 0.5s
- Agent execution: 2m 7s
- Total: 2m 15s

### Issues
[None / List any issues found]

### Recommendations
[Any suggestions for improvement]
```
