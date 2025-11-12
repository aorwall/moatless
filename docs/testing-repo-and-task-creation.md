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

2. **Select Repository**
   - Click the repository selector button (first button in the toolbar)
   - Search or select "hello-world-ui" from the list
   - If repository doesn't appear, check for API connection issues

3. **Enter Task Description**
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

4. **Configure Task Settings**
   - **Model**: Sonnet 4.5 (recommended)
   - **Mode**: Code (for implementation) or Edit Mode (for modifications)

5. **Create Task**
   - Click "Create Task" button
   - Wait for task to initialize

### Step 4: Monitor Task Execution

1. **Check Task Status**
   - Task should appear in the sidebar with:
     - Task name
     - Repository name
     - Timestamp

2. **Watch for Errors**

   **Common Errors to Check**:

   a) **JSON Parse Error**:
   ```
   "JSON Parse error: Unexpected identifier \"bun\""
   ```
   - **Cause**: Commands stored as plain strings instead of JSON arrays
   - **Fix**: Update database with JSON array format

   b) **Failed to Load Repository Configuration**:
   ```
   "Failed to load repository configuration"
   ```
   - **Cause**: API calls failing (ERR_CONNECTION_REFUSED)
   - **Check**: Browser console for network errors
   - **Check**: Network tab for failed API requests

   c) **Connection Errors**:
   - Check browser console: `ERR_CONNECTION_REFUSED`
   - Verify API endpoint accessibility
   - Test direct fetch: `fetch('/api/v1/repositories/{id}')`

### Step 5: Start Preview Server

1. **Navigate to Task Page**
   - Click on the created task in the sidebar

2. **Start Server**
   - Look for "Start Server" button in the preview panel
   - Click to start the development server
   - Wait for "Server is starting..." message

3. **Verify Server Started**
   - Preview iframe should load with the application
   - Check for any error messages in the preview area
   - Console should show server logs

4. **Test Inspector**
   - Click the "Toggle Inspector" button
   - Button should show active state
   - Inspector overlay should appear on the preview

### Step 6: Verify Agent Execution

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

### Step 7: Test Todo App Functionality

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

### Step 8: Test Auto-Reload

1. **Make a Code Change**
   - Ask agent to modify styling or add feature
   - Wait for file to save

2. **Verify Auto-Reload**
   - Preview should automatically refresh
   - Changes should appear without manual reload
   - Check console for HMR (Hot Module Reload) messages

### Step 9: Test Screenshots

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

## Known Issues

### 1. API Calls Fail in Task Context

**Issue**: API calls to `/api/v1/repositories/*` fail with "Failed to fetch" when made from task pages (`/tasks/*`)

**Status**: Known bug, requires codebase fix

**Workaround**: Use direct database updates for repository configuration

**Files Involved**:
- `/opt/moatless-vibe/apps/frontend/src/features/repositories/hooks/use-repositories.ts`
- `/opt/moatless-vibe/apps/frontend/src/features/repositories/api/repositories.ts`

### 2. Repository Dialog Shows "Failed to load repository configuration"

**Issue**: Opening repository configuration dialog from task page shows error

**Cause**: Related to Issue #1 above

**Workaround**: Configure repositories from dashboard instead of task pages

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
