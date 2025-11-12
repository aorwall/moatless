# Test Data

Sample data for E2E tests.

## Task Descriptions

### Todo App - Full Featured
Use this for comprehensive task creation tests:

```
Create a todo app with the following features: Add new todos with a text input and submit button, Display list of todos with checkboxes, Mark todos as complete/incomplete by clicking checkbox, Delete todos with a delete button, Show count of active todos, Filter todos by: All, Active, Completed, Use Tailwind CSS for styling, Use TypeScript for type safety
```

### Todo App - Basic
Use this for quick tests:

```
Create a simple todo app with add, complete, and delete functionality. Use the existing React setup and Tailwind CSS for styling.
```

### Todo App - With Persistence
Use this for testing more complex requirements:

```
Create a todo app with: Add todos with text input, Mark complete/incomplete with checkboxes, Delete todos, Filter by All/Active/Completed, Show active count, LocalStorage persistence, Responsive design with Tailwind, TypeScript types for todo items
```

## Repository Configurations

### hello-world-ui (React + Vite + Tailwind)
- **URL**: `https://github.com/aorwall/hello-world-ui.git`
- **Setup**: `["bun install"]`
- **Server**: `["bun run dev"]`
- **Screenshots**: `true`
- **Auto-reload**: `true`
- **Inspector**: `true`

Expected to be pre-configured in most test environments.

## Expected Outcomes

### Todo App Screenshot Elements

When verifying todo app screenshots, look for:

1. **Header**: "Todo App" or similar heading
2. **Input Field**: Text input with placeholder (e.g., "What needs to be done?")
3. **Add Button**: Submit button (often labeled "Add")
4. **Filter Buttons**: Three buttons labeled "All", "Active", "Completed"
5. **Count Display**: Text showing remaining todos (e.g., "0 items left", "3 items left")
6. **Styling**:
   - Gradient background (typically blue to indigo)
   - Modern card layout with shadows
   - Rounded corners
   - Professional Tailwind CSS styling

### Timing Expectations

Based on production testing:

- **Task creation via API**: < 1 second
- **Agent starts processing**: Immediately if message provided
- **Agent completes task**: 1-3 minutes typical, up to 5 minutes acceptable
- **Screenshot availability**: Within 10 seconds of task completion
- **Preview server start**: 10-30 seconds depending on dependencies

### API Response Examples

**Repository List Response**:
```json
{
  "repositories": [
    {
      "id": "uuid-here",
      "name": "hello-world-ui",
      "codeExecution": {
        "setupCommands": ["bun install"]
      },
      "previewConfig": {
        "serverStartCommands": ["bun run dev"],
        "takeScreenshots": true,
        "autoReload": true,
        "enableInspector": true
      }
    }
  ]
}
```

**Task Creation Response**:
```json
{
  "taskId": "abc-123-uuid",
  "status": "running",
  "message": "Task created successfully"
}
```

**Task Status Response** (completed):
```json
{
  "id": "abc-123-uuid",
  "status": "open",
  "numTurns": 1,
  "iterations": [
    {
      "iteration_number": 1,
      "status": "completed",
      "initialMessage": "Create a todo app...",
      "screenshot_file_id": "screenshot-uuid",
      "screenshot_path": "/path/to/screenshot.png"
    }
  ]
}
```
