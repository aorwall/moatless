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
- **Quick validation after edits**: Run `pnpm post-edit-hook` for fast formatting fixes and type checking (~100ms with caching)

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

For detailed LLM instructions, see [Git Operations Patterns](docs/patterns/git-operations-guide.md).

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
All API calls should use TanStack Query hooks for consistent caching, error handling, and loading states.

**📌 Pattern Reference**: For advanced React Query patterns including optimistic updates, infinite queries, and complex caching strategies, see [React Query Patterns](docs/patterns/react-query.md).

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
Query keys are centralized in `src/lib/api/query-keys.ts`.

**📌 Pattern Reference**: For smart cache invalidation strategies based on events and data dependencies, see [Cache Invalidation Pattern](docs/patterns/cache-invalidation.md).
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
The task detail page uses a React Context to manage and share task-related data across components.

**📌 Pattern Reference**: For performance-optimized context management with separated concerns, see [Tri-Context Pattern](docs/patterns/tri-context.md).

**Context Structure (`/features/tasks/contexts/task-context.tsx`)**:
- Centralizes all task data fetching (metadata, events, git diff, workspace status)
- Manages SSE connections for real-time updates
- Handles UI state (selected iteration, panel views)
- Provides error and loading states

**📌 Pattern References**: 
- For SSE connection management and reconnection logic, see [SSE Management Pattern](docs/patterns/sse-management.md)
- For event-driven updates across components, see [Event-Driven Architecture](docs/patterns/event-driven.md)

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

**📌 Pattern Reference**: For comprehensive error handling strategies including recovery mechanisms and error boundary composition, see [Error Handling Pattern](docs/patterns/error-handling.md).

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

**📌 Pattern Reference**: For computing values without effects and avoiding unnecessary state, see [Derived State Pattern](docs/patterns/derived-state.md).

### Best Practices

1. **API Integration**:
   - Always use React Query hooks instead of direct API calls
   - Implement optimistic updates for better UX
   - Use proper error boundaries

**📌 Pattern References**: 
- For API design and implementation patterns, see [API Best Practices](docs/patterns/api-best-practices.md)
- For schema design with Zod, see [Schema Design Patterns](docs/patterns/schema-design-patterns.md)

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

The backend follows a specific testing strategy with proper naming conventions to ensure fast development cycles. For detailed backend testing patterns, see [Backend Testing Patterns](docs/patterns/backend-testing.md).

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
This project uses a layered testing strategy that balances speed, reliability, and comprehensive coverage.

**📌 Pattern Reference**: For strategic mocking approaches and testing best practices, see [Strategic Mocking Pattern](docs/patterns/strategic-mocking.md).

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

#### Testing Patterns
- Use `.test.ts` or `.test.tsx` file extensions
- Test behavior, not implementation details
- **Prioritize integration over isolation** for route and page components
- Mock strategically - only what's necessary for test stability
- Aim for >80% coverage on critical paths
- Always test loading states, error states, and edge cases
- Use contract tests to verify component interfaces
- **Note**: `any` types are allowed in test files (`**/*.test.ts`, `**/*.test.tsx`, `**/__tests__/**/*`)

### Comprehensive Testing Patterns
For detailed testing patterns, mocking strategies, and best practices, see the [Frontend Testing Patterns](docs/patterns/frontend-testing.md).

## Architectural Patterns Quick Reference

This section helps you quickly identify which architectural patterns to use for specific tasks. These patterns are battle-tested solutions to common challenges in this codebase.

### When to Use Each Pattern

| **Task/Problem** | **Pattern to Use** | **Key Indicators** | **Documentation** |
|------------------|-------------------|--------------------|--------------------|
| Managing multiple React contexts with performance issues | Tri-Context Pattern | - Components re-rendering unnecessarily<br>- Need to separate data, actions, and UI state<br>- Performance degradation with context updates | [docs/patterns/tri-context.md](docs/patterns/tri-context.md) |
| Implementing real-time features with SSE | SSE Management Pattern | - Need server-sent events<br>- Real-time data updates<br>- Building chat or live features | [docs/patterns/sse-management.md](docs/patterns/sse-management.md) |
| Managing server state and API calls | React Query Patterns | - Fetching data from APIs<br>- Need caching and synchronization<br>- Implementing mutations | [docs/patterns/react-query.md](docs/patterns/react-query.md) |
| Invalidating caches based on events | Smart Cache Invalidation | - Complex data dependencies<br>- Event-driven updates<br>- Need selective cache updates | [docs/patterns/cache-invalidation.md](docs/patterns/cache-invalidation.md) |
| Building event-driven features | Event-Driven Architecture | - Decoupled component communication<br>- Central event handling<br>- Cross-feature notifications | [docs/patterns/event-driven.md](docs/patterns/event-driven.md) |
| Computing values from props/state | Derived State Pattern | - Tempted to use useEffect for calculations<br>- Need computed values<br>- Dependent calculations | [docs/patterns/derived-state.md](docs/patterns/derived-state.md) |
| Handling errors gracefully | Error Boundary Strategies | - Need fallback UI<br>- Component error isolation<br>- Recovery mechanisms | [docs/patterns/error-handling.md](docs/patterns/error-handling.md) |
| Writing effective tests | Strategic Mocking Pattern | - Testing complex components<br>- Balance between speed and coverage<br>- Integration vs unit tests | [docs/patterns/strategic-mocking.md](docs/patterns/strategic-mocking.md) |
| Testing backend services | Backend Testing Patterns | - Testing Node.js services<br>- Need fast/slow test separation<br>- Testing with file system operations | [docs/patterns/backend-testing.md](docs/patterns/backend-testing.md) |
| Testing frontend components | Frontend Testing Patterns | - Testing React components<br>- Component integration tests<br>- Mocking strategies | [docs/patterns/frontend-testing.md](docs/patterns/frontend-testing.md) |
| E2E testing with Playwright | Playwright E2E Testing | - End-to-end test scenarios<br>- Browser automation<br>- User journey testing | [docs/patterns/playwright-e2e-testing-guide.md](docs/patterns/playwright-e2e-testing-guide.md) |
| Styling components | Styling Patterns | - CSS-in-JS decisions<br>- Tailwind CSS usage<br>- Component styling strategies | [docs/patterns/styling-guide.md](docs/patterns/styling-guide.md) |
| Handling git operations as an LLM | Git Operations Guide | - User requests to commit changes<br>- User wants to push to main<br>- LLM handling git worktrees | [docs/patterns/git-operations-guide.md](docs/patterns/git-operations-guide.md) |
| Designing APIs and schemas | API & Schema Patterns | - Building new endpoints<br>- Defining data structures<br>- Type-safe validation | [docs/patterns/api-best-practices.md](docs/patterns/api-best-practices.md)<br>[docs/patterns/schema-design-patterns.md](docs/patterns/schema-design-patterns.md) |

### Pattern Decision Tree

```
Are you building a new feature?
├─ Yes → Does it need real-time updates?
│   ├─ Yes → Use SSE Management Pattern
│   └─ No → Does it fetch data from APIs?
│       ├─ Yes → Use React Query Patterns
│       └─ No → Continue to state management
│
├─ Are you fixing performance issues?
│   ├─ Context re-rendering → Use Tri-Context Pattern
│   ├─ Unnecessary calculations → Use Derived State Pattern
│   └─ Cache problems → Use Smart Cache Invalidation
│
└─ Are you handling errors?
    ├─ Component crashes → Use Error Boundary Strategies
    └─ API errors → See React Query error handling
```

### Common Pattern Combinations

1. **Real-time Task Features**: SSE Management + Event-Driven + Cache Invalidation
2. **Data-Heavy Pages**: React Query + Tri-Context + Derived State
3. **Complex Forms**: React Query mutations + Error Handling + Event-Driven updates
4. **Performance Optimization**: Tri-Context + Derived State + Strategic component splitting

### Pattern Anti-Patterns to Avoid

- **DON'T** use `useEffect` for derived values → Use Derived State Pattern
- **DON'T** manually manage loading/error states → Use React Query Patterns
- **DON'T** put all state in one context → Use Tri-Context Pattern
- **DON'T** catch errors with try/catch everywhere → Use Error Boundary Strategies
- **DON'T** mock everything in tests → Use Strategic Mocking Pattern

