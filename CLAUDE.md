# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

### Development Commands 
```bash
# Install dependencies (requires pnpm 10.7.1)
pnpm install

# Development (runs all apps concurrently with Turborepo)
pnpm dev

# Build for production (builds all packages and apps)
pnpm build

# Run tests (tiered by speed and scope)
pnpm test             # standard tests (excludes E2E)
pnpm test:unit        # unit tests only (fastest)
pnpm test:integration # integration tests only
pnpm test:e2e         # E2E tests only (slowest)
pnpm test:fast        # fast tests for git hooks
pnpm test:all         # all tests including E2E
pnpm test:ui          # with UI and coverage
pnpm test:coverage    # with coverage report

# Type checking
pnpm type:check

# Linting and formatting
pnpm biome:check      # check for issues
pnpm biome:fix        # auto-fix issues

# Run all checks (optimized for speed with Turborepo)
pnpm check:turbo

# Install shadcn/ui components (run from frontend app)
cd apps/frontend && pnpm dlx shadcn@latest add [component]
```

### Running on Different Ports
```bash
# Run backend on a different port (useful for testing)
PORT=8001 pnpm start

# Run frontend dev server on a different port
PORT=3001 pnpm dev

# Environment variables for different ports
PORT=8001 NODE_ENV=production pnpm start  # Backend production on port 8001
PORT=8001 pnpm dev                         # Development on port 8001
```

## Mandatory Workflow
- You MUST run `pnpm type:check` after code changes to catch TypeScript errors
- You MUST run `pnpm biome:check` after code changes for linting and formatting
- You can run both `biome:fix` and `biome:check`
- For comprehensive validation, run `pnpm check:turbo` which includes all checks (optimized for speed)

## Commit Conventions

### Commit Message Format
This project uses conventional commit format with commitlint enforcement:

```
<type>: <description>

[optional body]

[optional footer(s)]
```

**Important:** The header (first line) must be ≤100 characters due to commitlint rules.

### Commit Types
- `feat:` - New features
- `fix:` - Bug fixes
- `docs:` - Documentation changes
- `style:` - Code style changes (formatting, missing semicolons, etc.)
- `refactor:` - Code refactoring without feature changes
- `test:` - Adding or updating tests
- `chore:` - Maintenance tasks, dependency updates

### Scope Guidelines
The project enforces specific commit scopes via commitlint:
- `components` - UI component changes
- `layout` - Layout-related changes
- `routes` - Route/page-level changes
- `styles` - Styling changes
- `utils` - Utility function changes
- `hooks` - React hook changes
- Or omit the scope entirely: `feat: description` instead of `feat(scope): description`

**Note**: Commits with scopes outside this list will be rejected by commitlint.

### Examples
```bash
# Good commit messages
feat: add global temporary DATA_DIR setup for all tests
fix: resolve memory leak in task event streaming
docs: update testing guidelines in CLAUDE.md
test: add integration tests for environment service

# With scope
feat(components): add loading skeleton for task list
feat(utils): add environment service with file search
fix(hooks): handle edge case in validation hooks
```

### Git Worktree Workflow (for LLMs)

**This workflow MUST ALWAYS be followed when making any commits in this worktree environment.**

When handling git operations in this worktree environment:

1. **For commits (always executed):**
   ```bash
   git status
   git add .
   git commit -m "feat: [description]"  # Keep header ≤100 chars
   ```

2. **For pushing to main (automatically follows every commit):**
   ```bash
   # Handle generated files first
   git checkout -- *.gen.ts
   
   # Update and rebase
   git fetch origin main
   git rebase origin/main
   
   # Push to main
   git push origin HEAD:main
   ```

3. **Handle conflicts (when they occur):**
   - Unstaged changes: `git stash push -u -m "WIP"` then retry
   - Merge conflicts: Fix conflicts, `git add .`, `git rebase --continue`
   - Hook failures: Use `git push origin HEAD:main --no-verify`

**Important:** Always discard generated files (`*.gen.ts`) before rebasing to avoid conflicts.

**Note:** Every commit will automatically trigger the rebase and push workflow to keep main updated.

For detailed LLM instructions, see [Git Operations Guide](docs/guidelines/git-operations-guide.md).

### Pre-commit Hooks
- **pre-push**: Runs fast tests (`pnpm test:fast`) before pushing
- **commit-msg**: Validates commit message format using commitlint

## Architecture

For a comprehensive overview of the system architecture, see [Architecture Overview](./docs/architecture-overview.md).

### Project Structure (Turborepo Monorepo)
```
├── apps/                      # Application packages
│   ├── frontend/              # React frontend application
│   │   ├── src/
│   │   │   ├── features/      # Feature-based modules
│   │   │   │   ├── claude-code/   # Claude Code integration
│   │   │   │   │   ├── api/         # Claude Code API
│   │   │   │   │   ├── hooks/       # Claude Code hooks
│   │   │   │   │   └── index.ts     # Feature exports
│   │   │   │   ├── environments/  # Environment management
│   │   │   │   │   ├── api/         # Environment API endpoints
│   │   │   │   │   ├── components/  # Environment components
│   │   │   │   │   ├── hooks/       # Environment hooks
│   │   │   │   │   └── index.ts     # Feature exports
│   │   │   │   ├── navigation/    # Navigation components
│   │   │   │   │   └── components/  # Sidebar, breadcrumbs, etc.
│   │   │   │   ├── preview/       # Preview functionality
│   │   │   │   │   ├── api/         # Preview API
│   │   │   │   │   ├── components/  # Preview UI components
│   │   │   │   │   ├── hooks/       # Preview hooks
│   │   │   │   │   ├── stores/      # Preview state management
│   │   │   │   │   ├── types/       # Preview types
│   │   │   │   │   ├── utils/       # Preview utilities
│   │   │   │   │   └── index.ts     # Feature exports
│   │   │   │   ├── settings/      # Settings management
│   │   │   │   │   ├── components/  # Settings components
│   │   │   │   │   └── hooks/       # Settings hooks
│   │   │   │   └── tasks/         # Tasks feature
│   │   │   │       ├── api/         # Task API endpoints
│   │   │   │       ├── components/  # Task-related components
│   │   │   │       ├── hooks/       # Task-specific hooks
│   │   │   │       ├── stores/      # Task state management
│   │   │   │       ├── utils/       # Task utilities
│   │   │   │       └── index.ts     # Feature exports
│   │   │   ├── lib/           # Shared utilities and components
│   │   │   │   ├── api/       # Core API utilities
│   │   │   │   │   ├── client.ts    # Base HTTP client
│   │   │   │   │   ├── query-client.ts # React Query setup
│   │   │   │   │   └── query-keys.ts   # Query key factory
│   │   │   │   ├── components/    # Shared components
│   │   │   │   │   ├── ui/          # Generic UI components
│   │   │   │   │   ├── error-boundary.tsx # Error handling
│   │   │   │   │   └── theme-provider.tsx # Theme management
│   │   │   │   ├── hooks/     # Shared hooks
│   │   │   │   ├── layout/    # Layout components
│   │   │   │   ├── styles/    # Global styles
│   │   │   │   └── utils/     # Utility functions
│   │   │   ├── routes/        # Page components & routing
│   │   │   │   ├── repositories/  # Repository routes
│   │   │   │   ├── tasks/         # Task routes
│   │   │   │   └── settings/      # Settings route
│   │   │   ├── test-utils/    # Testing utilities
│   │   │   ├── types/         # TypeScript type definitions
│   │   │   │   └── git-diff.ts    # Git diff types (only remaining)
│   │   │   └── main.tsx      # Application entry point
│   │   ├── dist/             # Build output
│   │   ├── package.json      # Frontend package configuration
│   │   └── vite.config.ts    # Vite configuration
│   └── backend/              # Node.js backend application
│       ├── routes/           # API route handlers
│       ├── services/         # Business logic services
│       │   └── agents/       # AI agent implementations
│       ├── plugins/          # Hono plugins
│       ├── utils/            # Server utilities
│       ├── app.ts           # Server entry point
│       ├── index.ts         # Main entry point
│       ├── dist/            # Build output
│       └── package.json     # Backend package configuration
├── packages/                 # Shared packages
│   └── types/              # Shared code between frontend and backend
│       ├── types/           # Shared TypeScript types
│       ├── constants/       # Shared constants
│       ├── schemas/         # Validation schemas
│       ├── utils/           # Shared utilities
│       ├── index.ts         # Package exports
│       └── package.json     # Shared package configuration
├── data/                    # Persistent data storage (created at runtime)
│   ├── events/                # Task events
│   ├── repositories/          # Repository configurations
│   ├── tasks/                 # Task metadata
│   └── workspaces/            # Workspace data
└── docs/                      # Documentation
```


### Technology Stack
- **Monorepo Tool**: Turborepo
- **Package Manager**: pnpm workspaces
- **Frontend Framework**: React 19 with TypeScript
- **Backend Framework**: Hono with TypeScript
- **Build Tools**: Vite (frontend), TypeScript Compiler (backend)
- **Styling**: Tailwind CSS v4
- **Routing**: TanStack Router (file-based)
- **Data Fetching**: TanStack Query (React Query)
- **Code Quality**: Biome
- **Testing**: Vitest

### Monorepo Workflow
- **Types Package**: `@moatless/types` contains types, constants, schemas, and utilities shared between frontend and backend
- **Development**: `pnpm dev` runs all apps concurrently using Turborepo
- **Building**: `pnpm build` builds all packages in the correct order (types → apps)
- **Testing**: Tests run with Turborepo caching for optimal performance
- **Type Safety**: Shared types ensure consistency across frontend and backend

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
- **Import Organization**: Biome automatically organizes imports when running `pnpm biome:fix`
- **Generated Files**: `routeTree.gen.ts` is excluded from linting/formatting (auto-generated by TanStack Router)
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
  - Shared imports: `@moatless/types` (from the types package)
  - Library imports: `@/lib/`
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
  - Use types API types from `@types/`
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
# Run all tests (excludes E2E by default)
pnpm test

# Run only unit tests (fastest)
pnpm test:unit

# Run integration tests
pnpm test:integration

# Run E2E tests (slowest)
pnpm test:e2e

# Run fast tests (for git hooks)
pnpm test:fast

# Run all tests including E2E
pnpm test:all

# Run tests with coverage
pnpm test:coverage

# Run tests with UI
pnpm test:ui
```

### Backend Testing Strategy

The backend follows a specific testing strategy with proper naming conventions to ensure fast development cycles. For detailed backend testing guidelines, see [Backend Testing Guidelines](docs/guidelines/backend-testing.md).

#### Test Naming Conventions
- **Unit Tests**: `*.test.ts` or `*.unit.test.ts` (fast, <200ms)
- **Integration Tests**: `*.integration.test.ts` (moderate, 200ms-2s)
- **E2E Tests**: `*.e2e.test.ts` (slow, >2s)

#### Test Performance Tiers
**FAST Tests (<200ms)** - Run on every push:
- Pure unit tests with mocked dependencies
- No file system operations
- No git operations
- No server/process spawning

**MODERATE Tests (200ms-2s)** - Run in CI:
- Integration tests with real file system
- Git operation tests
- Database tests

**SLOW Tests (>2s)** - Run separately:
- E2E tests with real servers
- Process lifecycle tests
- Preview environment tests

#### Test Commands Usage
```bash
# Development - Fast feedback loop
pnpm test:unit

# Pre-push hook - Essential coverage
pnpm test:fast

# CI Pipeline - Full integration testing
pnpm test

# Nightly/Release - Complete validation
pnpm test:all
```

#### Git Hook Optimization
The `.husky/pre-push` hook uses `pnpm test:fast` which excludes integration and E2E tests, reducing push time to ~30 seconds while maintaining essential test coverage.

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
vi.mock('@/lib/features/ui/resizable');

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
- **Note**: `any` types are allowed in test files (`**/*.test.ts`, `**/*.test.tsx`, `**/__tests__/**/*`)

### Comprehensive Testing Guide
For detailed testing patterns, mocking strategies, and best practices, see the [Testing Guide](docs/guidelines/frontend-testing.md).

