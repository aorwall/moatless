# E2E Test: Repository and Task Creation

Execute an end-to-end test of the Moatless Vibe application, testing repository configuration verification and task creation with a todo app example.

## Test Overview

This skill automates the test case documented in `docs/testing-repo-and-task-creation.md`, which tests:
- Repository configuration verification
- Task creation workflow
- Agent execution and completion
- Screenshot capture and verification
- Preview server functionality

## Prerequisites

Before running this test:
- Moatless Vibe instance must be running
- Access to the dashboard at `/`
- The `hello-world-ui` repository should be configured (or will be verified/created)

## Test Execution Steps

You will execute the following test workflow:

### Step 1: Verify Repository Configuration

1. **Navigate to Dashboard**
   - Access the preview at `/`
   - Take a snapshot to verify the dashboard loads correctly

2. **Check Repository Configuration**
   - Use curl or API to fetch repository list: `GET /api/v1/repositories`
   - Verify `hello-world-ui` repository exists
   - Confirm configuration:
     - `codeExecution.setupCommands`: `["bun install"]`
     - `previewConfig.serverStartCommands`: `["bun run dev"]`
     - `previewConfig.takeScreenshots`: `true`
     - `previewConfig.autoReload`: `true`
     - `previewConfig.enableInspector`: `true`

3. **If repository missing or misconfigured**:
   - Document the issue
   - Refer user to manual setup instructions in the testing guide

### Step 2: Create Task via API

**Important**: Use the API to create tasks, NOT the UI typeText tool, to avoid the duplicate task issue caused by Enter key events.

1. **Prepare Task Description**
   ```
   Create a todo app with the following features: Add new todos with a text input and submit button, Display list of todos with checkboxes, Mark todos as complete/incomplete by clicking checkbox, Delete todos with a delete button, Show count of active todos, Filter todos by: All, Active, Completed, Use Tailwind CSS for styling, Use TypeScript for type safety
   ```

2. **Create Task via API**
   ```bash
   curl -X POST http://localhost:PORT/api/v1/tasks \
     -H "Content-Type: application/json" \
     -d '{
       "agentType": "claude-code",
       "repositoryId": "REPO-ID-HERE",
       "message": "Create a todo app with the following features: Add new todos with a text input and submit button, Display list of todos with checkboxes, Mark todos as complete/incomplete by clicking checkbox, Delete todos with a delete button, Show count of active todos, Filter todos by: All, Active, Completed, Use Tailwind CSS for styling, Use TypeScript for type safety",
       "model": "claude-sonnet-4-5",
       "agentMode": "edit"
     }'
   ```

3. **Capture Task ID** from the response for subsequent verification

### Step 3: Verify Single Task Created

**Critical Check**: Ensure no duplicate tasks were created

1. Query recent tasks:
   ```bash
   curl http://localhost:PORT/api/v1/tasks?limit=20 | jq '.tasks[0:5]'
   ```

2. Check for duplicates:
   - Look for multiple tasks with similar descriptions
   - Check creation timestamps (should not be within seconds of each other)
   - If duplicates found, document the issue

### Step 4: Monitor Task Execution

1. **Navigate to Task Page**
   - Use `mcp__moatless-tools__navigatePreview` to go to `/tasks/{taskId}`
   - Take initial snapshot

2. **Poll Task Status**
   ```bash
   curl http://localhost:PORT/api/v1/tasks/{taskId} | jq '{status: .status, numTurns: .numTurns}'
   ```

3. **Wait for Completion**
   - Poll every 10-15 seconds
   - Monitor for status change from `running` → `open`
   - Timeout after 5 minutes (agent should complete in 1-3 minutes typically)
   - Track timing: record start and completion times

4. **Verify Agent Activity**
   - Check that `numTurns` > 0
   - Verify `iterations` array has at least one entry
   - Confirm `iterations[0].initialMessage` matches the task description

### Step 5: Verify Screenshot Captured

1. **Check Task Details**
   ```bash
   curl http://localhost:PORT/api/v1/tasks/{taskId} | jq '.iterations[0] | {screenshot_file_id, screenshot_path}'
   ```

2. **Navigate to Dashboard**
   - Go back to `/`
   - Locate the newly created task in the task list

3. **Verify Screenshot Presence**
   - Check that a screenshot preview is visible on the task card
   - If possible, use `mcp__moatless-tools__takePreviewScreenshot` or check the screenshot file

4. **Screenshot Verification Checklist**:
   - ✅ Todo app interface is visible
   - ✅ Input field with placeholder text present
   - ✅ "Add" button visible
   - ✅ Filter buttons (All, Active, Completed) present
   - ✅ Todo count display visible (e.g., "0 items left")
   - ✅ Styled with Tailwind CSS (gradient background, modern design)

### Step 6: Verify Preview Server

1. **Check Server Status**
   ```bash
   curl http://localhost:PORT/api/v1/tasks/{taskId}/server/status
   ```

2. **Verify Server Running**
   - Status should indicate server is running
   - Port should be assigned
   - No errors in server logs

## Success Criteria

Report test results against these criteria:

- ✅ Repository configuration verified (correct setup and server commands)
- ✅ Single task created successfully (no duplicates)
- ✅ Task created via API without errors
- ✅ Agent started processing immediately
- ✅ Agent completed within expected time (1-3 minutes)
- ✅ Task status changed from "running" to "open"
- ✅ Preview server started automatically
- ✅ Screenshot captured showing todo app
- ✅ Screenshot shows all required UI elements
- ✅ Task card displays screenshot preview on dashboard

## Error Handling

If any step fails:

1. **Document the failure**:
   - Which step failed
   - Error message received
   - Current state of the system

2. **Check common issues**:
   - JSON parse errors → Check repository configuration format
   - Task not starting → Verify repository ID and API endpoint
   - No screenshot → Check `takeScreenshots` setting in repository config
   - Duplicate tasks → Note if created via UI vs API

3. **Provide troubleshooting guidance**:
   - Reference the troubleshooting section in `docs/testing-repo-and-task-creation.md`
   - Suggest API commands to diagnose the issue
   - Recommend next steps for manual verification

## Test Report Format

Provide a comprehensive test report with:

1. **Test Summary**
   - Overall result: PASS/FAIL
   - Execution time
   - Number of steps completed

2. **Detailed Results**
   - Each step with PASS/FAIL status
   - Task ID created
   - Timing measurements (task creation, agent completion)
   - Screenshot verification results

3. **Issues Found**
   - Any failures or warnings
   - Performance concerns (e.g., agent took >3 minutes)
   - Duplicate tasks detected

4. **Recommendations**
   - Any suggested improvements
   - Configuration issues to address

## Notes

- **Always use API for task creation**, never UI automation with typeText
- The test is based on `docs/testing-repo-and-task-creation.md` - refer to it for additional context
- This test does NOT perform interactive testing (clicking, typing in the todo app) - it only verifies the screenshot
- If preview server is already running from a previous test, that's expected behavior
- Screenshots may take a few seconds after task completion to be captured and available

## Example Test Execution

```bash
# 1. Check repository
curl http://localhost:50843/api/v1/repositories | jq '.repositories[] | select(.name == "hello-world-ui")'

# 2. Get repository ID
REPO_ID=$(curl -s http://localhost:50843/api/v1/repositories | jq -r '.repositories[] | select(.name == "hello-world-ui") | .id')

# 3. Create task
TASK_RESPONSE=$(curl -s -X POST http://localhost:50843/api/v1/tasks \
  -H "Content-Type: application/json" \
  -d "{
    \"agentType\": \"claude-code\",
    \"repositoryId\": \"$REPO_ID\",
    \"message\": \"Create a todo app with the following features: Add new todos with a text input and submit button, Display list of todos with checkboxes, Mark todos as complete/incomplete by clicking checkbox, Delete todos with a delete button, Show count of active todos, Filter todos by: All, Active, Completed, Use Tailwind CSS for styling, Use TypeScript for type safety\",
    \"model\": \"claude-sonnet-4-5\",
    \"agentMode\": \"edit\"
  }")

# 4. Extract task ID
TASK_ID=$(echo $TASK_RESPONSE | jq -r '.taskId')

# 5. Monitor until complete
while true; do
  STATUS=$(curl -s http://localhost:50843/api/v1/tasks/$TASK_ID | jq -r '.status')
  echo "Task status: $STATUS"
  if [ "$STATUS" = "open" ]; then
    break
  fi
  sleep 15
done

# 6. Verify completion
curl http://localhost:50843/api/v1/tasks/$TASK_ID | jq '{status, numTurns, screenshot: .iterations[0].screenshot_file_id}'
```
