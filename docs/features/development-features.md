# Development Features

## Quick Reference

### Key Files
- **Error Boundary**: `src/lib/components/error-boundary.tsx`
- **Task Creation Dialog**: `src/features/tasks/components/create-task-dialog.tsx`
- **Root Setup**: `src/routes/__root.tsx`

### Debug Mode Setup
```javascript
// Enable debug mode in browser console
localStorage.setItem('moatlessDebugMode', 'true')
localStorage.setItem('repositoryId', '0ee846ce-fabd-4b90-9ec5-def401a5d802')
```

## Error Boundary with Task Creation

### Architecture
```mermaid
graph TB
    ERR[Runtime Error] --> EB[Error Boundary]
    EB --> CHK{Debug Mode?}
    CHK -->|Yes| BTN[Show "Create Task" Button]
    CHK -->|No| RST[Show "Try Again" Only]
    BTN --> DLG[Task Creation Dialog]
    DLG --> TSK[Pre-filled Error Task]
```

### Implementation Guide

#### 1. Basic Error Boundary Usage
```tsx
// Simple protection
<ErrorBoundary componentName="Feature Component">
  <FeatureComponent />
</ErrorBoundary>

// With task creation
<ErrorBoundary 
  componentName="Chat View"
  enableTaskCreation={true}
  repositoryId={task?.repositoryId}
>
  <ChatView />
</ErrorBoundary>
```

#### 2. Advanced Features
```tsx
<ErrorBoundary 
  componentName="Dynamic Component"
  resetKeys={[userId, dataVersion]} // Auto-reset on prop change
  onError={(error, info) => trackError(error)} // Custom error handling
  isolate={true} // Prevent error propagation
  fallback={(error, reset) => <CustomErrorUI />} // Custom UI
>
  <DynamicComponent />
</ErrorBoundary>
```

#### 3. Task Message Format
```
Fix this error in [Component Name]:

Error: [error message]

Stack trace:
[full stack trace]

Please investigate and fix this issue.
```

### Error Boundary Hierarchy
```
Root ErrorBoundary (catches all)
├── Chat View (chat errors)
├── Event Viewer (event errors)
├── Git Diff Viewer (diff errors)
└── Feature Components (feature-specific)
```

## Decision Trees

### When to Use Error Boundaries
```
Component might throw errors?
├─ Critical path? → Wrap with ErrorBoundary
├─ Async data loading? → Use SafeBoundary (includes Suspense)
├─ Dynamic content? → Add resetKeys
└─ Need error tracking? → Add onError callback
```

### Debug Mode Features
```
Want to enable debug features?
├─ Set moatlessDebugMode → 'true'
├─ Set repositoryId → valid repo ID
├─ enableTaskCreation → true on boundary
└─ Check both are strings, not booleans
```

## Common Patterns

### SafeBoundary for Async Components
```tsx
import { SafeBoundary } from '@/lib/components/error-boundary';

<SafeBoundary
  componentName="Async Component"
  loadingFallback={<Skeleton />}
>
  <AsyncComponent />
</SafeBoundary>
```

### Feature-Level Error Isolation
```tsx
// Prevents one feature's errors from crashing others
<ErrorBoundary componentName="Feature A" isolate>
  <FeatureA />
</ErrorBoundary>
<ErrorBoundary componentName="Feature B" isolate>
  <FeatureB />
</ErrorBoundary>
```

## Troubleshooting

| Issue | Check | Solution |
|-------|-------|----------|
| No "Create Task" button | localStorage values | Ensure both are strings: `'true'` not `true` |
| Repository not selected | Repository ID exists | Verify ID in system |
| Errors still crash app | Boundary placement | Add boundaries at feature level |
| Reset not working | Reset keys | Pass changing props to resetKeys |