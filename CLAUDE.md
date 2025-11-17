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

### Pre-commit Hooks
- **pre-push**: Runs fast tests (`bun run test:fast`) before pushing
- **commit-msg**: Validates commit message format using commitlint

## Skills System

This project uses Claude Code's skills system to provide comprehensive development guidance. Skills are automatically activated based on the files you're working on or keywords in your requests.

### Available Skills

#### frontend-dev-guidelines
**Automatically activated when working on**: Frontend code (`apps/frontend/src/**/*.tsx`, `*.ts`)

**Covers**: React 19 patterns, TanStack Query/Router, Tailwind v4, shadcn/ui, component design, data fetching, routing, state management, derived state, error handling, performance optimization, TypeScript patterns, testing

**Use this skill when**:
- Creating or modifying React components
- Implementing data fetching with TanStack Query
- Working with routes and pages (TanStack Router)
- Styling components with Tailwind CSS
- Managing state or computing derived values
- Optimizing performance
- Writing frontend tests

#### backend-dev-guidelines
**Automatically activated when working on**: Backend code (`apps/backend/src/**/*.ts`, `packages/types/**/*.ts`)

**Covers**: Hono + OpenAPI, InversifyJS dependency injection, Drizzle ORM, layered architecture (routes→services→repositories), schema validation with Zod, API contract design, error handling, TypeScript patterns, testing

**Use this skill when**:
- Creating new API endpoints
- Implementing business logic in services
- Working with database repositories
- Defining API contracts and schemas
- Setting up dependency injection
- Writing backend tests

#### testing-guidelines
**Automatically activated when working on**: Test files (`**/*.test.ts`, `**/*.test.tsx`, `**/__tests__/**/*`)

**Covers**: Vitest patterns, React Testing Library, strategic mocking, test tier classification (Tier 1/2/3), unit vs integration tests, coverage goals, test organization

**Use this skill when**:
- Writing any type of test
- Deciding what to mock
- Determining test coverage priorities
- Organizing test suites

### How Skills Work

1. **Automatic Activation**: Skills are automatically suggested when you work on files matching their patterns or use relevant keywords
2. **On-Demand Access**: Ask Claude to "use the [skill-name] skill" to access detailed guidance
3. **Contextual Help**: Skills provide detailed patterns, code examples, and anti-patterns
4. **Consistency**: Skills ensure all developers follow the same patterns

### Skill vs CLAUDE.md

- **CLAUDE.md** (this file): Commands, workflows, commit conventions, project structure, high-level principles
- **Skills**: Detailed implementation patterns, code examples, best practices, anti-patterns, comprehensive guidance

**When in doubt**: Consult the appropriate skill for detailed implementation guidance.

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
- **Backend Runtime**: Bun (development and production)
- **Build Tools**: Vite (frontend), TypeScript Compiler (backend for production builds)
- **Module System**: ESNext with bundler resolution (no `.js` extensions required)
- **Styling**: Tailwind CSS v4
- **Routing**: TanStack Router (file-based)
- **Data Fetching**: TanStack Query (React Query)
- **Code Quality**: Biome
- **Testing**: Vitest

### API Integration

All API calls use TanStack Query hooks for consistent caching, error handling, and loading states. Query keys are centralized in `src/lib/api/query-keys.ts` using a hierarchical structure for efficient cache invalidation.

**📌 See frontend-dev-guidelines skill**: [Data Fetching](resources/data-fetching.md) for implementation patterns, optimistic updates, infinite queries, and cache invalidation strategies.

### Development Guidelines

**IMPORTANT: Consult the appropriate skill for detailed implementation patterns:**

| Working On... | Skill to Use | Key Topics |
|---------------|--------------|------------|
| React components, pages, UI | **frontend-dev-guidelines** | Components, hooks, data fetching, routing, state, styling, performance |
| API endpoints, services, DB | **backend-dev-guidelines** | API contracts, DI, layered architecture, repositories, validation |
| Any type of test | **testing-guidelines** | Test strategy, mocking, tier classification, organization |

**General Development Principles:**
- **Path Aliases**: `@/*` maps to `./src/*` in TypeScript and Vite
- **File Naming**: Kebab-case convention enforced by Biome
- **Import Organization**: Biome automatically organizes imports when running `bun run biome:fix`
- **Generated Files**: `routeTree.gen.ts` is excluded from linting/formatting (auto-generated by TanStack Router)
- **Feature-Based Architecture**:
  - Features contain components, hooks, and API code
  - Shared utilities in `/lib/`
  - Page logic lives in `/routes/` files

#### Component Organization

- **Route Files** (`/routes/*.tsx`): Handle data fetching, page-level state, layout composition
- **Feature Components** (`/features/*/components/`): Handle UI implementation, interactions, reusable logic
- **Shared Components** (`/lib/components/`): Generic, reusable UI components

**📌 See frontend-dev-guidelines skill**: [Component Patterns](resources/component-patterns.md) and [Routing](resources/routing.md) for detailed guidance.

#### Application Architecture

- **Task Context Pattern**: React Context manages task-related data, SSE connections, and UI state
- **Event-Driven Updates**: Real-time updates via Server-Sent Events (SSE)
- **Feature-Based Organization**: Features contain components, hooks, API code, and state

**📌 See docs/architecture-overview.md** for detailed architecture patterns.

#### Critical Development Rules

**Type Safety** (NEVER violate these):
  - Use TypeScript strict mode
  - **NEVER use `any` type** - always use proper types or `unknown`
  - **NEVER cast to `any`** - fix the underlying type issue instead
  - Use type guards and narrowing for runtime type checking

**State Management**:
  - Use TanStack Query for server state (NEVER duplicate in React state)
  - Use React state for local UI state only
  - Calculate derived values during render (don't use effects)

**Error Handling**:
  - Use error boundaries to catch rendering errors
  - Always handle loading and error states in data fetching
  - Display user-friendly error messages with recovery options

**📌 See Skills**: frontend-dev-guidelines for [Error Handling](resources/error-handling.md), [Derived State](resources/derived-state.md), [TypeScript Patterns](resources/typescript-patterns.md)

### Skill Usage Quick Reference

| Task | Skill | Resource |
|------|-------|----------|
| React components | frontend-dev-guidelines | Component Patterns, Routing |
| Data fetching | frontend-dev-guidelines | Data Fetching |
| State management | frontend-dev-guidelines | State Management, Derived State |
| Styling | frontend-dev-guidelines | Styling |
| API endpoints | backend-dev-guidelines | API Contract Pattern |
| Services/business logic | backend-dev-guidelines | Service Layer, Dependency Injection |
| Database access | backend-dev-guidelines | Repository Pattern |
| Validation/schemas | backend-dev-guidelines | Schema Validation |
| Any testing | testing-guidelines | Testing Philosophy, Strategic Mocking |

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

The backend follows a specific testing strategy with proper naming conventions to ensure fast development cycles.

**📌 See Skills**: backend-dev-guidelines [Testing](resources/testing.md) and testing-guidelines for detailed patterns.

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

### Testing Strategy

The project follows a strategic mocking approach that balances test speed with integration coverage. Key principles:
- Test behavior, not implementation details
- **Prioritize integration over isolation** for route and page components
- Mock strategically - only external dependencies
- Aim for >80% coverage on critical paths (Tier 1)
- Always test loading states, error states, and edge cases
- **Note**: `any` types are allowed in test files

**📌 See testing-guidelines skill**: [Strategic Mocking](resources/strategic-mocking.md), [Tier Classification](resources/tier-classification.md), and [Testing Philosophy](resources/testing-philosophy.md) for comprehensive guidance.
