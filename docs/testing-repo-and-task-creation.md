# Testing Guide: Repository Creation and Task Execution

## Overview
This manuscript documents the process of testing repository creation and task execution in Moatless Vibe, using a todo app creation task as the test case.

**Important**: Always use the UI to create and configure repositories. The UI automatically handles proper data formatting (converting text input to JSON arrays). SQL queries should only be used for verification, not for creating or modifying repository configurations.

## Prerequisites
- Moatless Vibe instance running
- Access to the dashboard at `/`
- A GitHub repository URL (or local path) to test with
- Browser with developer tools for debugging

## Test Case: Create Todo App from React Template

### Step 1: Create a New Repository

1. **Navigate to Repositories**
   - From any page, click "Repositories" in the sidebar
   - Or from the dashboard, access via settings

2. **Add Repository**
   - Click "New Repository" or "Add Repository" button
   - Enter repository URL: `https://github.com/aorwall/hello-world-ui.git`
   - Click "Create Repository"
   - The system will automatically:
     - Detect the repository type (GitHub/GitLab/local)
     - Create the repository entry
     - Show the configuration form

3. **Configure Repository Settings**

   After creation, the repository configuration form appears with these sections:

   **General Settings**:
   - Repository name (auto-filled from URL)
   - Description (optional)
   - Remote URL (the GitHub/GitLab URL)

   **Code Execution**:
   - **Setup Commands**: Enter commands one per line (the UI will convert to array)
     ```
     bun install
     ```

   **Preview Environment**:
   - **Start Commands**: Enter commands one per line
     ```
     bun run dev
     ```
   - **Take Screenshots**: ✅ Enable
   - **Auto-Reload**: ✅ Enable
   - **Enable Inspector**: ✅ Enable

4. **Save Configuration**
   - Make changes to any field (the Save button enables when changes are detected)
   - Click "Save Changes"
   - Verify success toast message appears
   - The configuration is saved with commands as JSON arrays in the database

### Step 2: Verify Repository Configuration

**Database Verification** (if direct access available):
```sql
SELECT
  id,
  name,
  setup_commands,
  server_start_commands,
  take_screenshots,
  auto_reload,
  enable_inspector
FROM repositories
WHERE name = 'hello-world-ui';
```

**Expected Output**:
- `setup_commands`: `["bun install"]`
- `server_start_commands`: `["bun run dev"]`
- `take_screenshots`: `1`
- `auto_reload`: `1`
- `enable_inspector`: `1`

### Step 3: Create a New Task

1. **Navigate to Dashboard**
   - Return to the main dashboard (`/`)
   - You should see a large text input area with placeholder "Describe your coding task or leave empty to just run the app..."

2. **Select Repository**
   - Click the repository selector button (shows current repository, e.g., "hello-world-ui")
   - If no repository is selected, click to open the dropdown
   - Search or select "hello-world-ui" from the list
   - If repository doesn't appear, check for API connection issues

3. **Enter Task Description (Initial Message)**

   **Important**: Type your task description directly in the text area. This message will be sent to the agent automatically when the task is created, and the agent will start processing immediately.

   Example task description:
   ```
   Create a todo app with the following features:
   - Add new todos with a text input and submit button
   - Display list of todos with checkboxes
   - Mark todos as complete/incomplete by clicking checkbox
   - Delete todos with a delete button
   - Show count of active todos
   - Filter todos by: All, Active, Completed
   - Use Tailwind CSS for styling
   - Use TypeScript for type safety
   ```

   **Note**: You can also leave the text area empty to create a task without an initial message. The agent will wait for you to send the first message from the task page.

4. **Configure Task Settings**
   - **Branch**: Select a branch to work from (optional, defaults to main)
   - **Model**: Sonnet 4.5 (default and recommended)
   - **Agent Mode**:
     - **Edit Mode** (default): Agent makes code changes directly
     - **Plan Mode**: Agent creates implementation plans

   **Optional**: Click the attachment button (📎) to:
   - Upload files for context
   - Import designs from Figma

5. **Create Task**
   - Click "Create Task" button (with send icon)
   - You will be automatically redirected to the task page (`/tasks/{taskId}`)
   - **If you entered a message**: The agent will start processing immediately
   - **If you left it empty**: You can send your first message from the task page

### Step 3a: Understanding Task Status States

After task creation, the task will have one of these statuses:

- **`open`**: Task is ready to receive messages. Agent is not currently running.
- **`running`**: Agent is actively processing a message.
- **`error`**: Task encountered an error during execution.
- **`merged`**: Task changes have been merged to the main branch.
- **`closed`**: Task has been closed and workspace cleaned up.

**How to check task status**:
```sql
SELECT task_id, name, status, num_turns FROM tasks WHERE task_id = '{your-task-id}';
```

- `num_turns = 0`: No messages have been processed yet
- `num_turns > 0`: Agent has processed messages

**Reference**: Task status schema is defined in `packages/types/src/schemas/base.schemas.ts`

### Step 4: Monitor Task Execution

**If you sent an initial message**, the agent should start processing immediately:

1. **Observe Agent Activity**
   - You should see messages appearing in the chat area
   - Look for "thinking" indicator while agent processes
   - Agent will use various tools (Read, Write, Edit, Bash, etc.)
   - Tool calls will appear inline showing the agent's actions

2. **Check Task Status Changes**
   - Status changes from `open` → `running` when agent starts
   - Status returns to `open` when agent completes
   - Task appears in sidebar with updated timestamp

3. **Monitor Preview Server**
   - Preview server starts automatically if needed
   - Watch for "Server is starting..." message in preview panel
   - Preview iframe will show the application once server is ready
   - Auto-reload is enabled by default (changes appear automatically)

**If you didn't send an initial message**:
   - Task page shows: "No messages yet. Start a conversation!"
   - Preview server may start automatically (depending on repository config)
   - Send your first message using the chat input at the bottom

4. **Watch for Errors**

   **Common Errors to Check**:

   a) **JSON Parse Error**:
   ```
   "JSON Parse error: Unexpected identifier \"bun\""
   ```
   - **Cause**: Commands stored as plain strings instead of JSON arrays in database
   - **Fix**: Update database with JSON array format (see Troubleshooting section)

   b) **Connection Errors**:
   - Check browser console for `ERR_CONNECTION_REFUSED`
   - Verify API endpoint accessibility
   - Check event stream connection indicator (green dot in header)

   c) **Agent Not Starting**:
   - Verify task status: `SELECT status FROM tasks WHERE task_id = '{id}'`
   - Check if iteration was created: `SELECT * FROM task_iterations WHERE task_id = (SELECT id FROM tasks WHERE task_id = '{id}')`
   - If no iteration exists, message was not received (see Testing Best Practices)

### Step 5: Verify Agent Execution

1. **Monitor Agent Progress**
   - Agent should start reading repository files
   - Look for tool calls (e.g., "3 tool calls")
   - Check for file edits and creations

2. **Expected Agent Actions**:
   - Read existing `App.tsx`
   - Create or modify components for todo functionality
   - Update imports and types
   - Test the implementation

3. **Check Console Logs**
   - Click "Console" button to expand console panel
   - Look for build errors or warnings
   - Verify no runtime errors

### Step 6: Test Todo App Functionality

Once the agent completes:

1. **Visual Verification**
   - Preview should show todo app interface
   - Check for input field and submit button
   - Verify todo list renders

2. **Functional Testing**:

   a) **Add Todo**:
   - Type text in input field
   - Click submit or press Enter
   - Verify todo appears in list

   b) **Toggle Complete**:
   - Click checkbox next to todo
   - Verify visual state changes (strikethrough, different color)
   - Check todo moves between active/completed

   c) **Delete Todo**:
   - Click delete button
   - Verify todo is removed from list

   d) **Filter Todos**:
   - Click "All", "Active", "Completed" filters
   - Verify correct todos display

   e) **Count Display**:
   - Verify active todo count updates correctly

### Step 7: Test Auto-Reload

1. **Make a Code Change**
   - Ask agent to modify styling or add feature
   - Wait for file to save

2. **Verify Auto-Reload**
   - Preview should automatically refresh
   - Changes should appear without manual reload
   - Check console for HMR (Hot Module Reload) messages

### Step 8: Test Screenshots

1. **Check Screenshot Capture**
   - Screenshots should be automatically captured during agent actions
   - Look for screenshot indicators in the UI
   - Verify screenshots are stored and retrievable

## Troubleshooting Guide

### Issue: Repository Not Appearing in Selector

**Symptoms**:
- Repository selector shows "No repositories found"
- Recently created repository missing

**Diagnosis**:
1. Open browser DevTools (F12)
2. Check Console for errors
3. Check Network tab for API calls to `/api/v1/repositories`
4. Look for `ERR_CONNECTION_REFUSED` or 500 errors

**Solutions**:
- Verify API server is running
- Check database entry exists:
  ```sql
  SELECT * FROM repositories WHERE name = 'hello-world-ui';
  ```
- Test API endpoint directly:
  ```javascript
  fetch('http://localhost:65001/api/v1/repositories')
    .then(r => r.json())
    .then(console.log)
  ```

### Issue: JSON Parse Error on Task Page

**Symptoms**:
```
Error: HTTP_ERROR
"JSON Parse error: Unexpected identifier \"bun\""
```

**Root Cause**:
Commands stored as plain strings instead of JSON arrays in the database. This can happen if:
- Database was manually edited with incorrect format
- Migration or import script used wrong format
- Old version of the code created repositories differently

**How It Should Work**:
- The UI accepts commands as plain text (one per line)
- The UI automatically converts to JSON arrays when saving
- Database stores as JSON: `["bun install"]`
- UI converts back to plain text for display: `"bun install"`

**Fix** (if database has incorrect format):
```sql
-- Check current format
SELECT setup_commands, server_start_commands FROM repositories WHERE name = 'hello-world-ui';

-- If they're plain strings, fix them:
UPDATE repositories
SET
  setup_commands = '["bun install"]',
  server_start_commands = '["bun run dev"]',
  updated_at = strftime('%s', 'now')
WHERE name = 'hello-world-ui';
```

**Verification**:
```sql
SELECT setup_commands, server_start_commands FROM repositories WHERE name = 'hello-world-ui';
```

Expected output: `["bun install"]|["bun run dev"]`

**Prevention**:
- Always use the UI to create and edit repositories
- The UI handles the proper format conversion automatically
- Only use SQL for verification, not for configuration

### Issue: Preview Server Won't Start

**Symptoms**:
- "Start Server" button doesn't work
- Server stays in "starting" state
- Preview shows error message

**Diagnosis**:
1. Check server logs:
   - Click "Server" button in preview toolbar
   - Look for error messages
2. Verify setup commands ran successfully
3. Check if port is already in use

**Solutions**:
- Restart preview server using MCP tool
- Check repository setup commands are correct
- Verify dependencies installed correctly
- Check for port conflicts

### Issue: Agent Not Responding

**Symptoms**:
- No tool calls appear
- Agent stuck on "thinking"
- No progress after several minutes

**Solutions**:
- Check browser console for WebSocket errors
- Verify event stream connection (check status indicator)
- Refresh the page
- Check server logs for backend errors

## Database Schema Reference

### repositories table

```sql
CREATE TABLE repositories (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  repo_path TEXT,
  setup_commands TEXT,  -- JSON array: ["command1", "command2"]
  server_start_commands TEXT,  -- JSON array: ["command"]
  take_screenshots INTEGER DEFAULT 0,
  auto_reload INTEGER DEFAULT 1,
  enable_inspector INTEGER DEFAULT 0,
  created_at INTEGER,
  updated_at INTEGER
);
```

## API Endpoints Reference

### List Repositories
```
GET /api/v1/repositories
```

### Get Repository
```
GET /api/v1/repositories/{id}
```

### Create Repository
```
POST /api/v1/repositories
Content-Type: application/json

{
  "repo_path": "https://github.com/owner/repo.git",
  "setup_commands": ["bun install"],
  "server_start_commands": ["bun run dev"],
  "take_screenshots": true,
  "auto_reload": true,
  "enable_inspector": true
}
```

### Update Repository
```
PUT /api/v1/repositories/{id}
Content-Type: application/json

{
  "setup_commands": ["bun install"],
  "server_start_commands": ["bun run dev"]
}
```

### Create Task
```
POST /api/v1/tasks
Content-Type: application/json

{
  "agentType": "claude-code",
  "repositoryId": "repo-id-here",
  "message": "Your initial prompt to the agent (optional)",
  "model": "claude-sonnet-4-5",
  "agentMode": "edit",
  "originalBranch": "main",
  "fileIds": [],
  "contexts": []
}
```

**Notes**:
- If `message` is provided, the agent will start processing immediately
- If `message` is omitted or empty, the task is created in `open` status waiting for first message
- `agentMode`: `"edit"` (makes changes) or `"plan"` (creates plans)

### Send Message to Task
```
POST /api/v1/tasks/{taskId}/messages
Content-Type: application/json

{
  "message": "Your message to the agent",
  "resume": true,
  "agentMode": "edit",
  "model": "claude-sonnet-4-5",
  "fileIds": [],
  "contexts": []
}
```

**Notes**:
- Use this endpoint to send additional messages after task creation
- `resume`: If `true`, continues from previous state. If `false`, starts fresh.
- Task must be in `open` status (not `running`) to accept new messages

**Schema Reference**: `packages/types/src/schemas/task.schemas.ts:SendMessageRequestSchema`

### Get Task
```
GET /api/v1/tasks/{taskId}
```

Returns full task details including iterations, messages, and status.

### List Tasks
```
GET /api/v1/tasks?limit=50&offset=0
```

Returns paginated list of tasks.

## Testing Best Practices

### Manual Testing vs Automated Testing

When testing the UI, it's important to understand how React handles form inputs:

**✅ Correct Approach - Manual User Input**:
- Type directly into text fields
- Click buttons naturally
- React's synthetic event system properly updates component state

**❌ Incorrect Approach - JavaScript DOM Manipulation**:
```javascript
// This WILL NOT work correctly with React
textbox.value = 'some text';
textbox.dispatchEvent(new Event('input'));
```

**Why**: React maintains its own internal state separate from the DOM. Setting `element.value` directly doesn't update React's state, so when you submit a form, React will send its internal state (which is still empty) rather than what you see in the DOM.

**For Automated Testing**: Use proper testing libraries that know how to interact with React:
- Playwright with proper user simulation
- React Testing Library
- Cypress with proper commands

### Verifying Task Creation

After creating a task, verify it was created correctly:

```sql
-- Check task was created with message
SELECT
  task_id,
  name,
  description,
  status,
  num_turns
FROM tasks
WHERE task_id = '{your-task-id}';

-- Check if initial iteration was created
SELECT
  iteration_number,
  status,
  initial_message
FROM task_iterations
WHERE task_id = (
  SELECT id FROM tasks WHERE task_id = '{your-task-id}'
);
```

If `num_turns = 0` and no iterations exist, the message was not received by the backend.

## Known Issues

### 1. API Calls Fail in Task Context

**Issue**: API calls to `/api/v1/repositories/*` may fail with "Failed to fetch" when made from task pages (`/tasks/*`)

**Status**: Intermittent issue

**Workaround**: Configure repositories from dashboard (`/`) instead of task pages

**Files Involved**:
- `apps/frontend/src/features/repositories/hooks/use-repositories.ts`
- `apps/frontend/src/features/repositories/api/repositories.ts`

## Success Criteria

✅ Repository created successfully
✅ Repository appears in selector
✅ Task created without errors
✅ Task page loads without JSON errors
✅ Preview server starts
✅ Application renders in preview
✅ Todo app functionality works
✅ Auto-reload works on code changes
✅ Inspector toggles correctly
✅ Screenshots captured (if enabled)

## Test Data

### Sample Task Prompts

**Basic Todo App**:
```
Create a simple todo app with add, complete, and delete functionality.
Use the existing React setup and Tailwind CSS for styling.
```

**Feature-Rich Todo App**:
```
Create a todo app with:
- Add todos with text input
- Mark complete/incomplete with checkboxes
- Delete todos
- Filter by All/Active/Completed
- Show active count
- LocalStorage persistence
- Responsive design with Tailwind
- TypeScript types for todo items
```

**Todo App with Backend**:
```
Create a full-stack todo app:
- Frontend: React with TypeScript
- Add API endpoints for CRUD operations
- Use in-memory storage or SQLite
- Implement error handling
- Add loading states
```

## Appendix: Quick Reference Commands

### Database Queries

```sql
-- List all repositories
SELECT id, name, setup_commands, server_start_commands FROM repositories;

-- Check specific repository
SELECT * FROM repositories WHERE name = 'hello-world-ui';

-- Update commands to JSON format
UPDATE repositories
SET
  setup_commands = '["bun install"]',
  server_start_commands = '["bun run dev"]'
WHERE id = '{repository-id}';

-- Enable all features
UPDATE repositories
SET
  take_screenshots = 1,
  auto_reload = 1,
  enable_inspector = 1
WHERE name = 'hello-world-ui';
```

### Browser Console Commands

```javascript
// Test API connectivity
fetch('http://localhost:65001/api/v1/repositories')
  .then(r => r.json())
  .then(console.log)
  .catch(console.error);

// Test specific repository
fetch('http://localhost:65001/api/v1/repositories/{id}')
  .then(r => r.json())
  .then(console.log)
  .catch(console.error);

// Check WebSocket connection
// Look for connection status in UI or console logs
```

## Notes

- Always verify commands are stored as JSON arrays, not plain strings
- Check browser console for errors during repository creation
- Monitor network tab for failed API requests
- Use direct database queries when UI fails
- Test in both dashboard and task contexts to identify routing issues
- Screenshots and inspector features require proper configuration in database
- Auto-reload depends on the underlying dev server (Vite, webpack, etc.)

---

**Document Version**: 1.0
**Last Updated**: 2025-11-12
**Test Environment**: Moatless Vibe with Bun-based React repository
