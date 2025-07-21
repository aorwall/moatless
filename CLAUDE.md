# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

### Development Commands 
```bash
# Install dependencies (requires bun 10.7.1)
bun run install

# Development (runs all apps concurrently with Turborepo)
bun run dev

# Build for production (builds all packages and apps)
bun run build

# Run tests (tiered by speed and scope)
bun run test             # standard tests (excludes E2E)
bun run test:unit        # unit tests only (fastest)
bun run test:integration # integration tests only
bun run test:e2e         # E2E tests only (slowest)
bun run test:coverage    # with coverage report

# Type checking
bun run type:check

# Linting and formatting
bun run biome:check      # check for issues
bun run biome:fix        # auto-fix issues

# Run all checks (optimized for speed with Turborepo)
bun run check:turbo

# Install shadcn/ui components (run from frontend app)
cd apps/frontend && bunx shadcn@latest add [component]
```

### Running on Different Ports
```bash
# Run backend on a different port (useful for testing)
PORT=8001 bun run start

# Run frontend dev server on a different port
PORT=3001 bun run dev

# Environment variables for different ports
PORT=8001 NODE_ENV=production bun run start  # Backend production on port 8001
PORT=8001 bun run dev                         # Development on port 8001
```

## Mandatory Workflow
- You MUST run `bun run type:check` after code changes to catch TypeScript errors
- You MUST run `bun run biome:check` after code changes for linting and formatting
- You can run both `biome:fix` and `biome:check`
- For comprehensive validation, run `bun run check:turbo` which includes all checks (optimized for speed)
- **Quick validation after edits**: Run `bun run post-edit-hook` for fast formatting fixes and type checking (~100ms with caching)

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
- **pre-push**: Runs fast tests (`bun run test:fast`) before pushing
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
│   └── types/              # @moatless/types - Shared code between frontend and backend
│       ├── src/            # Source code (imported as @moatless/types)
│       │   ├── types/      # Shared TypeScript types
│       │   ├── constants/  # Shared constants
│       │   ├── schemas/    # Validation schemas
│       │   ├── utils/      # Shared utilities
│       │   └── index.ts    # Main exports
│       ├── dist/           # Build output
│       └── package.json    # Package configuration
├── data/                    # Persistent data storage (created at runtime)
│   ├── events/                # Task events
│   ├── repositories/          # Repository configurations
│   ├── tasks/                 # Task metadata
│   └── workspaces/            # Workspace data
└── docs/                      # Documentation
```


### Technology Stack
- **Monorepo Tool**: Turborepo
- **Package Manager**: bun workspaces
- **Frontend Framework**: React 19 with TypeScript
- **Backend Framework**: Hono with TypeScript
- **Build Tools**: Vite (frontend), TypeScript Compiler (backend)
- **Styling**: Tailwind CSS v4
- **Routing**: TanStack Router (file-based)
- **Data Fetching**: TanStack Query (React Query)
- **Code Quality**: Biome
- **Testing**: Vitest

### API Integration Patterns

#### Data Fetching with TanStack Query
All API calls should use TanStack Query hooks for consistent caching, error handling, and loading states.

**📌 Pattern Reference**: For implementation details, code examples, and advanced patterns including optimistic updates, infinite queries, and complex caching strategies, see [React Query Patterns](docs/patterns/react-query.md).

#### Query Key Management
Query keys are centralized in `src/lib/api/query-keys.ts`. The system uses a hierarchical key structure for efficient cache invalidation.

**📌 Pattern Reference**: For the complete query key reference, smart cache invalidation strategies, and implementation examples, see [Cache Invalidation Pattern](docs/patterns/cache-invalidation.md).

### Development Patterns & When to Use Them

**IMPORTANT: When implementing features, always check if there's a pattern for your use case:**

1. **UI Development**
   - Building settings/configuration screens? → Use [Configuration Dialog Pattern](docs/patterns/configuration-dialog.md)
   - Computing values from state? → Use [Derived State Pattern](docs/patterns/derived-state.md)

2. **API & Data Management**
   - Fetching data from APIs? → Use [React Query Patterns](docs/patterns/react-query.md)
   - Need to update caches? → Use [Cache Invalidation Pattern](docs/patterns/cache-invalidation.md)
   - Creating new endpoints? → Use [API Best Practices](docs/patterns/api-best-practices.md)
   - Validating data? → Use [Schema Design Patterns](docs/patterns/schema-design-patterns.md)

3. **Feature Development**
   - Working with tasks/agents? → Use [Task Lifecycle Pattern](docs/patterns/task-lifecycle.md)
   - Need cross-feature communication? → Use [Event-Driven Architecture](docs/patterns/event-driven.md)
   - Setting up backend services? → Use [Dependency Injection Pattern](docs/patterns/dependency-injection.md)

4. **Error Handling & Testing**
   - Implementing error handling in frontend? → Use [Error Handling Pattern](docs/patterns/error-handling.md)
   - Writing tests? → Check testing patterns (see Testing section)

**General Development Guidelines:**
- **Path Aliases**: `@/*` maps to `./src/*` in TypeScript and Vite
- **File Naming**: Kebab-case convention enforced by Biome
- **Import Organization**: Biome automatically organizes imports when running `bun run biome:fix`
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
The task detail page uses a React Context to manage and share task-related data across components. This pattern centralizes data fetching, SSE connections, and UI state management.

**📌 Pattern References**: 
- For complete implementation details and component structure, see [Tri-Context Pattern](docs/patterns/tri-context.md)
- For SSE connection management, see [SSE Management Pattern](docs/patterns/sse-management.md)
- For event-driven updates, see [Event-Driven Architecture](docs/patterns/event-driven.md)

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

Proper error handling and loading states are crucial for a good user experience. The application uses consistent patterns for handling errors, loading states, and empty data scenarios.

**📌 Pattern Reference**: For complete implementation patterns, code examples, and best practices for error handling and loading states, see [Error Handling Pattern](docs/patterns/error-handling.md).

#### Error Boundaries & Failure Handling

React Error Boundaries are essential for catching JavaScript errors in component trees and displaying fallback UI instead of crashing the entire application. The application uses error boundaries strategically to isolate failures and maintain a functional user experience.

**📌 Pattern Reference**: For comprehensive error handling strategies including code examples, recovery mechanisms, error boundary composition, and defensive programming patterns, see [Error Handling Pattern](docs/patterns/error-handling.md).

**Type Safety**:
  - Use types API types from `@types/`
  - Local types only for client-specific needs (e.g., git-diff)
  - Use TypeScript strict mode
  - **NEVER use `any` type** - always use proper types or `unknown` if type is truly unknown
  - **NEVER cast to `any`** - fix the underlying type issue instead
  - Use type guards and narrowing for runtime type checking

**State Management**:
  - Use TanStack Query for server state
  - Use React state for local UI state
  - Avoid redundant state storage

**📌 Pattern Reference**: For computing values without effects and avoiding unnecessary state, see [Derived State Pattern](docs/patterns/derived-state.md).

### Pattern Usage Guidelines

**When working on specific features, use these patterns:**

- **Building configuration/settings UI** → Use [Configuration Dialog Pattern](docs/patterns/configuration-dialog.md)
- **Managing tasks and agents** → Use [Task Lifecycle Pattern](docs/patterns/task-lifecycle.md)
- **Setting up backend services** → Use [Dependency Injection Pattern](docs/patterns/dependency-injection.md)
- **Implementing real-time features** → Use [SSE Management Pattern](docs/patterns/sse-management.md)
- **Working with API data** → Use [React Query Patterns](docs/patterns/react-query.md)
- **Optimizing React performance** → Use [Tri-Context Pattern](docs/patterns/tri-context.md)
- **Computing derived values** → Use [Derived State Pattern](docs/patterns/derived-state.md)
- **Handling errors** → Use [Error Handling Pattern](docs/patterns/error-handling.md)
- **Writing tests** → Use pattern appropriate to test type (see Testing section)

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

### Testing Strategy & Guidelines

#### Strategic Mocking Guidelines

The project follows a strategic mocking approach that balances test speed with integration coverage. The key principle is to mock external dependencies while testing real component integration.

**📌 Pattern Reference**: For detailed mocking guidelines, code examples, and best practices, see [Strategic Mocking Pattern](docs/patterns/strategic-mocking.md).

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
