# Testing Guide: Repository Creation and Task Execution

## Overview
This manuscript documents the process of testing repository creation and task execution in Moatless Vibe, using a todo app creation task as the test case.

**Important**: Always use the UI to create and configure repositories. The UI automatically handles proper data formatting (converting text input to JSON arrays). Use API endpoints for verification and debugging, not direct database access.

## Prerequisites
- Moatless Vibe instance running
- Access to the dashboard at `/`
- A GitHub repository URL (or local path) to test with
- Browser with developer tools for debugging

## Test Case: Create Todo App from React Template

### Step 1: Verify or Create Repository

**Note**: The `hello-world-ui` repository may already exist in your instance. Check the repository selector first before creating a new one.

1. **Check Existing Repositories**
   - From the dashboard (`/`), click the repository selector dropdown
   - Look for `hello-world-ui` in the list
   - If it exists, skip to Step 2 to verify its configuration

2. **Create Repository (if needed)**
   - From any page, click "Repositories" in the sidebar
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

**Via API**:

In browser console:
```javascript
// List all repositories
fetch('/api/v1/repositories')
  .then(r => r.json())
  .then(repos => {
    const repo = repos.find(r => r.name === 'hello-world-ui');
    console.log('Repository:', repo);
  });

// Or get specific repository by ID
fetch('/api/v1/repositories/{repo-id}')
  .then(r => r.json())
  .then(console.log);
```

Or using curl:
```bash
# List all repositories
curl http://localhost:PORT/api/v1/repositories | jq '.[] | select(.name == "hello-world-ui")'

# Get specific repository
curl http://localhost:PORT/api/v1/repositories/{repo-id}
```

**Expected Configuration**:
- `codeExecution.setupCommands`: `["bun install"]`
- `previewConfig.serverStartCommands`: `["bun run dev"]`
- `previewConfig.takeScreenshots`: `true`
- `previewConfig.autoReload`: `true`
- `previewConfig.enableInspector`: `true`

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

   **Important**: Type or paste your task description directly in the text area. This message will be sent to the agent automatically when the task is created, and the agent will start processing immediately.

   Example task description (paste as single line or use Shift+Enter for line breaks):
   ```
   Create a todo app with the following features: Add new todos with a text input and submit button, Display list of todos with checkboxes, Mark todos as complete/incomplete by clicking checkbox, Delete todos with a delete button, Show count of active todos, Filter todos by: All, Active, Completed, Use Tailwind CSS for styling, Use TypeScript for type safety
   ```

   **⚠️ Important Notes**:
   - You can also leave the text area empty to create a task without an initial message
   - Pressing Enter without Shift will submit the form and create the task immediately
   - For multi-line descriptions, use Shift+Enter or paste the text directly
   - The agent will start processing immediately after task creation if you provide a message

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
   - Click "Create Task" button (with send icon) **once**
   - ⚠️ **Do not click multiple times** - this will create duplicate tasks
   - Wait for the redirect to complete (button will show "Starting..." during creation)
   - You will be automatically redirected to the task page (`/tasks/{taskId}`)
   - **If you entered a message**: The agent will start processing immediately
   - **If you left it empty**: You can send your first message from the task page

**Expected Timing**:
- Task creation: < 1 second
- Agent starts processing: Immediately if message provided
- Typical task completion: 1-3 minutes depending on complexity

### Step 3a: Understanding Task Status States

After task creation, the task will have one of these statuses:

- **`open`**: Task is ready to receive messages. Agent is not currently running.
- **`running`**: Agent is actively processing a message.
- **`error`**: Task encountered an error during execution.
- **`merged`**: Task changes have been merged to the main branch.
- **`closed`**: Task has been closed and workspace cleaned up.

**How to check task status**:
```bash
# Via API
curl http://localhost:PORT/api/v1/tasks/{taskId}
```

Or in browser console:
```javascript
fetch('/api/v1/tasks/{taskId}').then(r => r.json()).then(console.log)
```

Look for these fields in the response:
- `status`: Current task status (`"open"`, `"running"`, etc.)
- `numTurns`: Number of messages processed (0 = no messages yet)
- `iterations`: Array of task iterations (empty if no messages sent)

**Reference**: Task status schema is defined in `packages/types/src/schemas/base.schemas.ts`

### Step 4: Monitor Task Execution

**Verify Only One Task Was Created**:
1. Check the sidebar task list
2. Look for duplicate tasks with similar descriptions
3. If duplicates exist (same description, created within seconds of each other):
   - Close the duplicate tasks by clicking the "Close task" button
   - This can happen if the "Create Task" button was clicked multiple times
   - Only one task should process your request

**If you sent an initial message**, the agent should start processing immediately:

1. **Observe Agent Activity**
   - You should see messages appearing in the chat area
   - Look for "thinking" indicator while agent processes
   - Agent will use various tools (Read, Write, Edit, Bash, etc.)
   - Tool calls will appear inline showing the agent's actions

2. **Check Task Status Changes**
   - Status changes from `open` → `running` when agent starts
   - Status returns to `open` when agent completes (typically 1-3 minutes)
   - Task appears in sidebar with updated timestamp
   - Button text changes from "Running" to "Send" when complete

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
   - Check task status via API: `fetch('/api/v1/tasks/{taskId}').then(r => r.json()).then(console.log)`
   - Look for `iterations` array in response - should contain at least one iteration if message was sent
   - Check `numTurns` field - should be > 0 if agent started processing
   - If `iterations` is empty and `numTurns` is 0, message was not received (see Testing Best Practices)

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

### Step 6: Verify Todo App in Screenshot

Once the agent completes (status changes to "open" and button shows "Send"):

1. **Check Task Screenshot**
   - Navigate back to the dashboard (`/`) or check the sidebar
   - Locate your task in the task list
   - A screenshot preview should be visible on the task card
   - Click "View full screenshot" to see the complete image

2. **Screenshot Verification Checklist**:
   - ✅ Todo app interface is visible
   - ✅ Input field with placeholder text present
   - ✅ "Add" button visible
   - ✅ Filter buttons (All, Active, Completed) present
   - ✅ Todo count display visible (e.g., "0 items left")
   - ✅ Styled with Tailwind CSS (gradient background, modern design)

3. **Optional: Manual Preview Testing**
   - If you want to interact with the app, view it directly in the preview iframe on the task page
   - The preview shows the live application with full interactivity

   **Note**: Direct interaction with preview elements via automated tools may not work due to iframe security restrictions. Manual browser interaction is the most reliable way to test functionality.

### Step 7: Optional - Test Auto-Reload

If you want to verify auto-reload functionality:

1. **Make a Code Change**
   - Send a follow-up message to the agent (e.g., "Change the background to purple")
   - Wait for the agent to make the code change
   - Watch the preview iframe

2. **Verify Auto-Reload**
   - Preview should automatically refresh when files change
   - Changes should appear without manual reload
   - Check console for HMR (Hot Module Reload) messages from Vite

**Note**: Auto-reload is enabled by default if configured in repository settings. It depends on the dev server's hot module replacement feature.

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
- Test API endpoint directly in browser console:
  ```javascript
  fetch('/api/v1/repositories')
    .then(r => r.json())
    .then(repos => {
      console.log('Repositories:', repos);
      console.log('Found hello-world-ui:', repos.some(r => r.name === 'hello-world-ui'));
    })
  ```
- Or using curl:
  ```bash
  curl http://localhost:PORT/api/v1/repositories | jq 'map(.name)'
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

**Diagnosis**:
Check the repository configuration via API:
```javascript
fetch('/api/v1/repositories')
  .then(r => r.json())
  .then(repos => {
    const repo = repos.find(r => r.name === 'hello-world-ui');
    console.log('Setup commands:', repo.codeExecution?.setupCommands);
    console.log('Server commands:', repo.previewConfig?.serverStartCommands);
  });
```

Commands should be arrays: `["bun install"]` not strings: `"bun install"`

**Fix**:
Use the repository configuration UI to re-save the commands:
1. Go to Repositories in sidebar
2. Click on hello-world-ui repository
3. Edit the Setup Commands and Server Start Commands fields
4. Click "Save Changes"
5. The UI will automatically convert to proper JSON array format

**Prevention**:
- Always use the UI to create and edit repositories
- The UI handles the proper format conversion automatically
- Avoid manual database edits

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

After creating a task, verify it was created correctly via the API:

```javascript
// In browser console
fetch('/api/v1/tasks/{your-task-id}')
  .then(r => r.json())
  .then(task => {
    console.log('Task status:', task.status);
    console.log('Messages processed:', task.numTurns);
    console.log('Iterations:', task.iterations.length);
    console.log('First iteration message:', task.iterations[0]?.initialMessage);
  });
```

Or using curl:
```bash
curl http://localhost:PORT/api/v1/tasks/{your-task-id} | jq '.status, .numTurns, .iterations'
```

**What to check**:
- `status`: Should be `"running"` if agent is processing, or `"open"` if waiting/completed
- `numTurns`: Should be > 0 if message was processed
- `iterations`: Should have at least one entry if initial message was sent
- `iterations[0].initialMessage`: Should contain your initial prompt

If `numTurns` is 0 and `iterations` is empty, the message was not received by the backend.

## Known Issues

### 1. API Calls Fail in Task Context

**Issue**: API calls to `/api/v1/repositories/*` may fail with "Failed to fetch" when made from task pages (`/tasks/*`)

**Status**: Intermittent issue

**Workaround**: Configure repositories from dashboard (`/`) instead of task pages

**Files Involved**:
- `apps/frontend/src/features/repositories/hooks/use-repositories.ts`
- `apps/frontend/src/features/repositories/api/repositories.ts`

## Success Criteria

✅ Repository created or verified successfully
✅ Repository appears in selector
✅ Repository configuration correct (setup commands, server commands, screenshots enabled)
✅ Single task created (no duplicates)
✅ Task created without errors
✅ Task page loads without JSON errors
✅ Agent starts processing immediately (if message provided)
✅ Agent completes within 1-3 minutes
✅ Task status changes from "running" to "open"
✅ Preview server starts automatically
✅ Screenshot captured showing todo app
✅ Screenshot shows all required UI elements (input, buttons, filters, count)
✅ Task card displays screenshot preview on dashboard

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

### API Commands (Browser Console)

```javascript
// List all repositories
fetch('/api/v1/repositories')
  .then(r => r.json())
  .then(repos => console.log('Repositories:', repos.map(r => r.name)))
  .catch(console.error);

// Get specific repository by name
fetch('/api/v1/repositories')
  .then(r => r.json())
  .then(repos => {
    const repo = repos.find(r => r.name === 'hello-world-ui');
    console.log('Repository config:', repo);
  });

// Get task details
fetch('/api/v1/tasks/{taskId}')
  .then(r => r.json())
  .then(task => {
    console.log('Status:', task.status);
    console.log('Turns:', task.numTurns);
    console.log('Iterations:', task.iterations);
  });

// List all tasks
fetch('/api/v1/tasks?limit=50')
  .then(r => r.json())
  .then(data => console.log('Tasks:', data.items));

// Send message to task
fetch('/api/v1/tasks/{taskId}/messages', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({
    message: 'Your message here',
    resume: true,
    agentMode: 'edit'
  })
}).then(r => r.json()).then(console.log);
```

### Curl Commands

```bash
# List repositories
curl http://localhost:PORT/api/v1/repositories | jq 'map(.name)'

# Get specific repository
curl http://localhost:PORT/api/v1/repositories/{repo-id} | jq .

# Get task details
curl http://localhost:PORT/api/v1/tasks/{taskId} | jq '.status, .numTurns'

# Send message to task
curl -X POST http://localhost:PORT/api/v1/tasks/{taskId}/messages \
  -H "Content-Type: application/json" \
  -d '{"message": "Your message", "resume": true, "agentMode": "edit"}'
```

## Notes

- Always verify commands are stored as JSON arrays via API, not plain strings
- Check browser console for errors during repository creation
- Monitor network tab for failed API requests
- Use API endpoints to verify configuration when UI shows errors
- Test in both dashboard and task contexts to identify routing issues
- Screenshots and inspector features require proper configuration in repository settings
- Auto-reload depends on the underlying dev server (Vite, webpack, etc.)
- Use browser console or curl with the API for debugging and verification

---

**Document Version**: 1.0
**Last Updated**: 2025-11-12
**Test Environment**: Moatless Vibe with Bun-based React repository
