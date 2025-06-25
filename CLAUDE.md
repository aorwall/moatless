# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

### Development Commands 
```bash
# Install dependencies (requires pnpm 10.7.1)
pnpm install

# Build for production
pnpm build

# Run tests
pnpm test
pnpm test:ui          # with UI and coverage
pnpm test:coverage    # with coverage report

# Type checking
pnpm type:check

# Linting and formatting
pnpm biome:check      # check for issues
pnpm biome:fix        # auto-fix issues

# Run all checks
pnpm check:turbo

# Install shadcn/ui components
pnpm dlx shadcn@latest add [component]
```

## Mandatory Workflow
- You MUST run `pnpm type:check` after code changes to catch TypeScript errors
- You MUST run `pnpm biome:check` after code changes for linting and formatting
- You can run both `biome:fix` and `biome:check`
- For comprehensive validation, run `pnpm check:turbo` which includes all checks

## Architecture

### Project Structure
```
├── client/                    # React frontend application
│   └── src/
│       ├── features/          # Feature-based modules
│       │   ├── claude-code/   # Claude Code integration
│       │   │   ├── api/         # Claude Code API
│       │   │   ├── hooks/       # Claude Code hooks
│       │   │   └── index.ts     # Feature exports
│       │   ├── environments/  # Environment management
│       │   │   ├── api/         # Environment API endpoints
│       │   │   ├── components/  # Environment components
│       │   │   ├── hooks/       # Environment hooks
│       │   │   └── index.ts     # Feature exports
│       │   ├── navigation/    # Navigation components
│       │   │   └── components/  # Sidebar, breadcrumbs, etc.
│       │   ├── preview/       # Preview functionality
│       │   │   ├── api/         # Preview API
│       │   │   ├── components/  # Preview UI components
│       │   │   ├── hooks/       # Preview hooks
│       │   │   ├── stores/      # Preview state management
│       │   │   ├── types/       # Preview types
│       │   │   ├── utils/       # Preview utilities
│       │   │   └── index.ts     # Feature exports
│       │   ├── settings/      # Settings management
│       │   │   ├── components/  # Settings components
│       │   │   └── hooks/       # Settings hooks
│       │   └── tasks/         # Tasks feature
│       │       ├── api/         # Task API endpoints
│       │       ├── components/  # Task-related components
│       │       ├── hooks/       # Task-specific hooks
│       │       ├── stores/      # Task state management
│       │       ├── utils/       # Task utilities
│       │       └── index.ts     # Feature exports
│       ├── lib/               # Shared utilities and components
│       │   ├── api/           # Core API utilities
│       │   │   ├── client.ts    # Base HTTP client
│       │   │   ├── query-client.ts # React Query setup
│       │   │   └── query-keys.ts   # Query key factory
│       │   ├── components/    # Shared components
│       │   │   ├── ui/          # Generic UI components
│       │   │   ├── error-boundary.tsx # Error handling
│       │   │   └── theme-provider.tsx # Theme management
│       │   ├── hooks/         # Shared hooks
│       │   ├── layout/        # Layout components
│       │   ├── styles/        # Global styles
│       │   └── utils/         # Utility functions
│       ├── routes/            # Page components & routing
│       │   ├── environments/  # Environment routes
│       │   ├── tasks/         # Task routes
│       │   └── settings/      # Settings route
│       ├── test-utils/        # Testing utilities
│       ├── types/             # TypeScript type definitions
│       │   └── git-diff.ts    # Git diff types (only remaining)
│       └── main.tsx          # Application entry point
├── server/                    # Node.js backend application
│   ├── routes/                # API route handlers
│   ├── services/              # Business logic services
│   │   └── agents/            # AI agent implementations
│   ├── plugins/               # Fastify plugins
│   ├── utils/                 # Server utilities
│   └── app.ts                # Server entry point
├── shared/                    # Shared code between client and server
│   ├── types/                 # Shared TypeScript types
│   ├── constants/             # Shared constants
│   ├── schemas/               # Validation schemas
│   └── utils/                 # Shared utilities
├── data/                      # Persistent data storage
│   ├── environments/          # Environment configurations
│   ├── events/                # Task events
│   ├── tasks/                 # Task metadata
│   └── workspaces/            # Workspace data
└── docs/                      # Documentation
```


### Technology Stack
- **Framework**: React 19 with TypeScript
- **Build Tool**: Vite
- **Styling**: Tailwind CSS v4
- **Routing**: TanStack Router (file-based)
- **Data Fetching**: TanStack Query (React Query)
- **Code Quality**: Biome
- **Testing**: Vitest

### API Integration Patterns

#### Data Fetching with TanStack Query
All API calls should use TanStack Query hooks for consistent caching, error handling, and loading states:

```typescript
// Use query hooks for GET requests
const { data, isLoading, error } = useTasksList({ activeOnly: true });

// Use mutation hooks for POST/PUT/DELETE
const createTask = useCreateTask({
  onSuccess: (data) => {
    // Handle success
  },
});
```

#### Query Key Management
Query keys are centralized in `src/lib/api/query-keys.ts`:
```typescript
// Task queries
queryKeys.tasks() // All task queries
queryKeys.task(taskId) // Specific task (invalidates all task data)
queryKeys.tasksList(filters) // Task list with filters
queryKeys.taskMessages(taskId, filters) // Task messages
queryKeys.taskEvents(taskId, filters) // Task events
queryKeys.taskGitDiff(taskId) // Git diff for task
queryKeys.taskFileSearch(taskId, query) // File search in task
queryKeys.taskWorkspaceStatus(taskId) // Workspace status
queryKeys.taskServerStatus(taskId) // Server status
queryKeys.taskServerLogs(taskId) // Server logs

// Environment queries
queryKeys.environments() // All environment queries
queryKeys.environmentsList() // Environment list
queryKeys.environmentDetail(envId) // Specific environment
queryKeys.environmentFileSearch(envId, query) // File search in environment
queryKeys.environmentBranches(envId) // Git branches for environment

// Agent queries
queryKeys.agents() // All agent queries
queryKeys.agentsList() // Agent list

// Claude Code queries
queryKeys.claudeCode.all() // All Claude Code queries
queryKeys.claudeCode.settings() // Claude Code settings
queryKeys.claudeCode.tools() // Claude Code tools
```

### Development Patterns

- **Path Aliases**: `@/*` maps to `./src/*` in TypeScript and Vite
- **File Naming**: Kebab-case convention enforced by Biome
- **Feature-Based Architecture**:
  - Features contain components, hooks, and API code
  - Shared utilities in `/lib/`
  - Page logic lives in `/routes/` files

#### Routing Strategy
Routes in this application follow a simple, direct pattern where route files handle all page-level concerns:

**Route Files (`/routes/*.tsx`)** handle:
- Data fetching using React Query hooks
- Page-level state management
- Error and loading states
- Layout composition and structure
- Orchestrating child components

**Feature Components (`/features/*/components/`)** handle:
- Specific UI implementation
- Component-level state
- User interactions and event handlers
- Reusable business logic

#### Task Context Pattern
The task detail page uses a React Context to manage and share task-related data across components:

**Context Structure (`/features/tasks/contexts/task-context.tsx`)**:
- Centralizes all task data fetching (metadata, events, git diff, workspace status)
- Manages SSE connections for real-time updates
- Handles UI state (selected iteration, panel views)
- Provides error and loading states

**Components Using Task Context**:
- `TaskHeader` - Displays task metadata and controls
- `TaskViewer` - Shows task iterations and results
- `TaskChatInput` - Wrapper for ChatInput that provides task context data
- `EventViewer` - Displays task events with real-time updates
- `GitDiffViewer` - Shows git diffs for tasks

**Note**: These components require TaskContext and must be used within the TaskProvider. The generic `ChatInput` component remains context-free and can be used in both task and non-task contexts.

- **Component Structure**: 
  - Use functional components with TypeScript
  - Colocate related components in feature folders
  - Export components as named exports
- **API Hooks**: 
  - Create custom hooks in feature-specific hooks folders
  - Include both query and mutation hooks
  - Handle loading, error, and success states
- **Import Patterns**:
  - Feature imports: `@/features/[feature]/`
  - Shared imports: `@/lib/`
  - Prefer named imports over barrel imports
- **Error Handling**: 
  - Use the `ApiError` class for consistent error handling
  - Display user-friendly error messages
  - Log errors to console for debugging

#### Error Handling & Loading States

**Component Error Handling Pattern:**
```typescript
// Always destructure error from React Query hooks
const { data, isLoading, error } = useTasksList();

// Handle errors first
if (error) {
  return <ErrorAlert error={error} />;
}

// Show skeleton loaders during loading
if (isLoading) {
  return <TaskListSkeleton />;
}

// Handle empty states
if (!data || data.length === 0) {
  return <EmptyState message="No tasks found" />;
}

// Render success state
return <TaskList tasks={data} />;
```

**Error Display Guidelines:**
- Use `ErrorAlert` component for consistent error display
- Extract error messages using `getErrorMessage` utility
- Never show generic "not found" when an API error occurs
- Distinguish between API failures and empty data

**Loading State Guidelines:**
- Use skeleton components instead of text loaders
- Match skeleton structure to actual content layout
- Keep skeleton animations subtle

**Common Patterns:**
- API Error: Show error alert with retry option
- Loading: Show content-appropriate skeleton
- Empty Data: Show helpful empty state with actions
- Not Found: Only show when data explicitly doesn't exist (no error)

#### Error Boundaries & Failure Handling

**Error Boundary Usage:**
React Error Boundaries are used to catch JavaScript errors in component trees, log errors, and display fallback UI instead of crashing the entire application.

```typescript
import { ErrorBoundary } from '@/lib/components/error-boundary';

// Basic usage
<ErrorBoundary componentName="Task Viewer">
  <TaskViewer />
</ErrorBoundary>

// With custom fallback
<ErrorBoundary
  componentName="Component Name"
  fallback={(error, reset) => (
    <div className="error-fallback">
      <p>Something went wrong</p>
      <button onClick={reset}>Try Again</button>
    </div>
  )}
>
  <ComponentThatMightError />
</ErrorBoundary>

// With automatic reset on prop changes
<ErrorBoundary
  componentName="Dynamic Component"
  resetKeys={[userId, dataVersion]}
  resetOnPropsChange={false}
>
  <ComponentWithDynamicData userId={userId} />
</ErrorBoundary>

// Combined with Suspense for loading states
import { SafeBoundary } from '@/lib/components/error-boundary-with-fallback';

<SafeBoundary
  componentName="Async Component"
  loadingFallback={<CustomSkeleton />}
>
  <AsyncComponent />
</SafeBoundary>
```

**Error Boundary Best Practices:**

1. **Granular Boundaries**: Place error boundaries strategically
   - Wrap feature modules (good granularity)
   - Don't wrap every single component (too granular)
   - Don't use just one at the root (not granular enough)

2. **Meaningful Error Messages**: Provide context-specific error information
   - Include component name for debugging
   - Show user-friendly messages
   - Log detailed errors for developers

3. **Recovery Mechanisms**:
   - Reset buttons for user-triggered recovery
   - Automatic reset on prop changes for dynamic content
   - Fallback UI that maintains layout structure

4. **Error Tracking Integration**:
   - Log errors to console in development
   - Send to error tracking service in production
   - Include component stack traces

**Failure Handling Strategy:**

1. **Component-Level Isolation**: Wrap each major feature component in an ErrorBoundary to isolate failures
   - Prevents cascading failures across the UI
   - Allows other parts of the application to remain functional
   - Provides contextual error messages

2. **Defensive Data Access**: Always use optional chaining and nullish coalescing for data access
   ```typescript
   // Good - defensive
   const value = data?.nested?.property ?? defaultValue;
   const usage = payload.usage || {};
   
   // Bad - can throw errors
   const value = data.nested.property;
   ```

3. **API Error Handling**: Use React Query's built-in error handling
   - Errors are automatically caught and returned in the `error` property
   - Implement retry logic for transient failures
   - Show user-friendly error messages

4. **Critical Path Protection**: Identify and protect critical user paths
   - Authentication flows
   - Data submission forms
   - Core navigation

5. **Error Recovery Actions**:
   - Provide "Try Again" buttons for retryable errors
   - Offer alternative actions when primary actions fail
   - Include helpful error messages that guide users

**What Error Boundaries DON'T Catch:**
- Event handler errors (use try/catch)
- Async errors (Promises, setTimeout)
- Server-side rendering errors
- Errors in the error boundary itself

**Best Practices:**
- Place boundaries around logical UI sections, not every component
- Never let errors bubble up to crash the entire app
- Log all errors to console for debugging (Error Boundaries do this automatically)
- Provide meaningful error messages to users
- Always include recovery options when possible
- Test error scenarios during development
- Consider using error tracking services (Sentry, LogRocket) in production

**Type Safety**:
  - Use shared API types from `@shared/types/`
  - Local types only for client-specific needs (e.g., git-diff)
  - Use TypeScript strict mode
  - Avoid `any` types

**State Management**:
  - Use TanStack Query for server state
  - Use React state for local UI state
  - Avoid redundant state storage

### Best Practices

1. **API Integration**:
   - Always use React Query hooks instead of direct API calls
   - Implement optimistic updates for better UX
   - Use proper error boundaries

2. **Performance**:
   - Leverage React Query's caching capabilities
   - Use query invalidation strategically
   - Implement pagination for large data sets

3. **Code Organization**:
   - Keep components small and focused
   - Extract reusable logic into custom hooks
   - Use barrel exports for cleaner imports

4. **Testing**:
   - Write tests for custom hooks
   - Mock API responses in tests
   - Test error states and edge cases

## Testing

### Overview
This project uses Vitest for unit and integration testing, with React Testing Library for component testing. Tests are organized in `__tests__` directories co-located with source code using the following structure:

```
src/features/tasks/
├── components/
│   ├── __tests__/
│   │   ├── task-viewer.test.tsx
│   │   └── task-header.test.tsx
│   └── task-viewer.tsx
├── hooks/
│   ├── __tests__/
│   │   └── use-tasks.test.ts
│   └── use-tasks.ts
```

### Quick Start
```bash
# Run all tests
pnpm test

# Run tests in watch mode
pnpm test:watch

# Run tests with coverage
pnpm test:coverage

# Run tests with UI
pnpm test:ui
```

### Testing Strategy & Guidelines

#### Layered Testing Approach
This project uses a layered testing strategy that balances speed, reliability, and comprehensive coverage:

1. **Unit Tests** - Individual component and hook testing with full mocking
2. **Integration Tests** - Route and feature testing with minimal strategic mocking  
3. **Contract Tests** - Verify component interfaces and dependency contracts
4. **E2E Tests** - Critical user journey testing (planned)

#### Strategic Mocking Guidelines

**DO Mock:**
- External dependencies (APIs, SSE connections, third-party libraries)
- Complex UI libraries that don't affect business logic (resizable panels, etc.)
- Error boundary and skeleton components
- Router and navigation hooks

**DO NOT Mock:**
- Feature components when testing integration
- Simple UI components (buttons, inputs, etc.)
- Components under test
- Business logic components

**Example - Route Integration Testing:**
```typescript
// ✅ Mock external dependencies only
vi.mock('@/features/tasks/hooks/use-tasks');
vi.mock('@/features/tasks/hooks/use-task-sse-connection');

// ✅ Mock complex UI libraries
vi.mock('@/lib/components/ui/resizable');

// ❌ Don't mock feature components - test integration!
// vi.mock('@/features/tasks/components/task-viewer');

// Test real component integration
it('displays task data through component hierarchy', () => {
  const taskData = createMockTaskWithIterations();
  vi.mocked(useTask).mockReturnValue({ data: taskData });
  
  render(<TaskDetail />);
  
  // Verify data flows through real components
  expect(screen.getByText(taskData.title)).toBeInTheDocument();
  expect(screen.getByText(taskData.iterations[0].message)).toBeInTheDocument();
});
```

#### Testing Guidelines
- Use `.test.ts` or `.test.tsx` file extensions
- Test behavior, not implementation details
- **Prioritize integration over isolation** for route and page components
- Mock strategically - only what's necessary for test stability
- Aim for >80% coverage on critical paths
- Always test loading states, error states, and edge cases
- Use contract tests to verify component interfaces

#### Benefits of Strategic Mocking
- **Catches Integration Bugs**: Real component interactions reveal interface mismatches
- **Detects Breaking Changes**: Changes to child components break parent tests
- **Validates Data Flow**: Ensures props and context flow correctly through component tree
- **Tests User Experience**: Closer to how users actually interact with the application

### Comprehensive Testing Guide
For detailed testing patterns, mocking strategies, and best practices, see the [Testing Guide](./docs/testing-guide.md).

