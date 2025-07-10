# Development Features

This document describes development-specific features and tools available in the Moatless Vibe repository to help contributors and developers working on the codebase.

## Error Boundary with Task Creation

The application includes an advanced error boundary system that allows developers to create tasks directly from runtime errors, making debugging and issue reporting seamless.

### Overview

When errors occur in the application, instead of showing a generic error message or crashing the entire app, the error boundary provides:
- User-friendly error display with component context
- Recovery options ("Try Again" button)
- **Debug mode**: Direct task creation from errors for developers

### Debug Mode Setup

The task creation feature is only available when debug mode is explicitly enabled via localStorage:

```javascript
// Enable debug mode
localStorage.setItem('moatlessDebugMode', 'true')

// Set the repository ID for task creation
localStorage.setItem('repositoryId', 'your-repository-id-here')

// Example with the main repository
localStorage.setItem('repositoryId', '0ee846ce-fabd-4b90-9ec5-def401a5d802')
```

### How It Works

1. **Error Detection**: React error boundaries catch JavaScript runtime errors and validation errors
2. **Context Preservation**: Error location, stack trace, and component name are captured
3. **Debug Mode Check**: System checks if `moatlessDebugMode` is enabled and `repositoryId` is set
4. **Task Creation**: If debug mode is active, shows "Create Task to Fix" button alongside "Try Again"
5. **Pre-filled Task**: Opens task creation dialog with comprehensive error information

### Error Boundary Hierarchy

```
Application Root (catches all unhandled errors)
├── Chat View (catches chat-specific errors)
├── Event Viewer (catches event-specific errors) 
├── Git Diff Viewer (catches diff-specific errors)
└── Other feature components...
```

### Task Creation Details

When creating a task from an error, the system automatically:
- Uses the repository ID from localStorage
- Pre-fills the task message with error context
- Sets mode to "code" for debugging tasks
- Includes full stack trace and component information

**Task Message Format:**
```
Fix this error in [Component Name]:

Error: [error message]

Stack trace:
[full stack trace]

Please investigate and fix this issue.
```

### Implementation Details

#### ErrorBoundary Component

Located at `src/lib/components/error-boundary.tsx`, the unified error boundary supports:

- **Basic error catching** - Prevents app crashes
- **Advanced features** - Reset keys, isolation mode, error callbacks
- **Task creation mode** - Controlled by `enableTaskCreation` prop
- **SafeBoundary wrapper** - Combines error boundary with Suspense for loading states

#### Usage Examples

**Basic Error Boundary:**
```tsx
<ErrorBoundary componentName="Feature Component">
  <FeatureComponent />
</ErrorBoundary>
```

**With Task Creation (Debug Mode):**
```tsx
<ErrorBoundary 
  componentName="Chat View"
  enableTaskCreation={true}
  repositoryId={task?.repositoryId}
>
  <ChatView />
</ErrorBoundary>
```

**Advanced Features:**
```tsx
<ErrorBoundary 
  componentName="Dynamic Component"
  resetKeys={[userId, dataVersion]}
  onError={(error, info) => trackError(error)}
  isolate={true}
>
  <DynamicComponent />
</ErrorBoundary>
```

### Troubleshooting

**Task creation button not showing:**
1. Verify `moatlessDebugMode` is set to `'true'` (string, not boolean)
2. Ensure `repositoryId` is set in localStorage
3. Check that both values are strings

**Repository not pre-selected:**
1. Confirm the repository ID exists in the system
2. Verify the repository selector is enabled
3. Check that `defaultRepositoryId` prop is being passed correctly

### Related Files

- `src/lib/components/error-boundary.tsx` - Main error boundary implementation
- `src/features/tasks/components/create-task-dialog.tsx` - Task creation dialog
- `src/features/tasks/components/chat-input/index.tsx` - Chat input with repository selection
- `src/routes/__root.tsx` - Root-level error boundary setup