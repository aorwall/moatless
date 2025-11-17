---
name: frontend-dev-guidelines
description: React 19 + TanStack Query/Router development patterns with Tailwind v4, shadcn/ui components, performance optimization, error handling, state management, data fetching, routing, styling, testing, component design, derived state, error boundaries, useMemo, useCallback, React.memo, query keys, mutations, cache invalidation, file-based routing, TypeScript, Vite build tool
---

# Frontend Development Guidelines

## Purpose
This skill provides comprehensive guidance for developing frontend features in the Moatless Vibe application using React 19, TanStack Query/Router, Tailwind v4, and shadcn/ui components.

## Technology Stack

### Core Technologies
- **React**: 19 with TypeScript
- **Data Fetching**: TanStack Query (React Query)
- **Routing**: TanStack Router (file-based)
- **Styling**: Tailwind CSS v4
- **UI Components**: shadcn/ui
- **Build Tool**: Vite
- **Testing**: Vitest + React Testing Library

### Key Libraries
- `@tanstack/react-query` - Server state management
- `@tanstack/react-router` - Type-safe routing
- `tailwindcss` - Utility-first CSS
- `@moatless/types` - Shared TypeScript types
- `@moatless/api-contract` - API contract definitions

## Quick Reference

| Need to... | See |
|------------|-----|
| Create a new component | [Component Patterns](resources/component-patterns.md) |
| Fetch data from API | [Data Fetching](resources/data-fetching.md) |
| Add a new route/page | [Routing](resources/routing.md) |
| Style components | [Styling](resources/styling.md) |
| Handle errors gracefully | [Error Handling](resources/error-handling.md) |
| Optimize performance | [Performance](resources/performance.md) |
| Document component states | [Storybook](resources/storybook.md) |
| Build configuration/settings UI | [Configuration Dialog](resources/configuration-dialog.md) |
| Write tests | [Testing](resources/testing.md) |
| Manage state | [State Management](resources/state-management.md) |
| Calculate derived values | [Derived State](resources/derived-state.md) |
| TypeScript patterns | [TypeScript Patterns](resources/typescript-patterns.md) |
| See complete examples | [Complete Examples](resources/complete-examples.md) |

## Core Principles

### 1. Feature-Based Architecture
Organize code by feature, not by technical layer:
```
src/features/tasks/
├── api/           # API endpoints and query keys
├── components/    # Task-specific components
├── hooks/         # Task-specific hooks
├── stores/        # Task state management (if needed)
├── utils/         # Task utilities
└── index.ts       # Feature exports
```

### 2. Type Safety First
- Use TypeScript strict mode
- NEVER use `any` type - use proper types or `unknown`
- Import types from `@moatless/types` for shared types
- Use type guards and narrowing for runtime checks

### 3. Server State with TanStack Query
- Use TanStack Query for ALL server state
- Never duplicate server state in React state
- Use centralized query keys from `src/lib/api/query-keys.ts`
- Implement optimistic updates for better UX

### 4. Component Design
- Single Responsibility Principle - describable in one sentence
- Prefer primitives over objects for props (exceptions: 5+ props)
- Use composition over configuration
- Co-locate state as close as possible to where it's used

### 5. Performance by Default
- Don't prematurely optimize
- Use `useMemo`, `useCallback`, `React.memo` strategically
- Avoid unnecessary re-renders
- Leverage React Query's caching

### 6. Error Handling
- Use error boundaries to catch rendering errors
- Display user-friendly error messages
- Provide recovery mechanisms when possible
- Always handle loading and error states in data fetching

### 7. Styling with Tailwind
- Use Tailwind utility classes
- Follow mobile-first responsive design
- Leverage shadcn/ui components for consistency
- Keep styles co-located with components

## Critical Anti-Patterns to Avoid

### TypeScript
❌ **NEVER** use `any` type
```tsx
// WRONG
const data: any = await fetchData()

// CORRECT
const data: TaskResponse = await fetchData()
// or if truly unknown
const data: unknown = await fetchData()
```

❌ **NEVER** use `import type`
```tsx
// WRONG
import type { Task } from '@moatless/types'

// CORRECT
import { Task } from '@moatless/types'
```

### State Management
❌ **DON'T** duplicate server state in React state
```tsx
// WRONG
const { data: tasks } = useQuery({ queryKey: ['tasks'] })
const [localTasks, setLocalTasks] = useState(tasks)

// CORRECT
const { data: tasks } = useQuery({ queryKey: ['tasks'] })
// Use tasks directly
```

❌ **DON'T** use effects for derived values
```tsx
// WRONG
const [fullName, setFullName] = useState('')
useEffect(() => {
  setFullName(`${firstName} ${lastName}`)
}, [firstName, lastName])

// CORRECT
const fullName = `${firstName} ${lastName}`
```

### Data Fetching
❌ **DON'T** hardcode API paths
```tsx
// WRONG
const response = await fetch('/api/v1/tasks')

// CORRECT
import { buildPath, routes } from '@moatless/api-contract'
const url = buildPath(routes.tasks.list)
const response = await fetch(url)
```

❌ **DON'T** forget error and loading states
```tsx
// WRONG
const { data } = useQuery({ queryKey: ['tasks'] })
return <div>{data.map(...)}</div>

// CORRECT
const { data, isLoading, error } = useQuery({ queryKey: ['tasks'] })
if (isLoading) return <Loading />
if (error) return <ErrorDisplay error={error} />
return <div>{data.map(...)}</div>
```

### Component Design
❌ **DON'T** pass entire objects when only primitives needed
```tsx
// WRONG
interface Props {
  task: Task
}
function TaskTitle({ task }: Props) {
  return <h1>{task.title}</h1>
}

// CORRECT
interface Props {
  title: string
}
function TaskTitle({ title }: Props) {
  return <h1>{title}</h1>
}
```

❌ **DON'T** prematurely abstract
```tsx
// WRONG - Only used once
function Button({ variant, size, onClick }) { ... }

// CORRECT - Wait for 3+ use cases
<button className="px-4 py-2 bg-blue-500" onClick={...}>
  Click me
</button>
```

## Project Structure Overview

```
apps/frontend/
├── src/
│   ├── features/              # Feature-based modules
│   │   ├── tasks/             # Task management feature
│   │   │   ├── api/           # Task API endpoints
│   │   │   ├── components/    # Task components
│   │   │   ├── hooks/         # Task hooks
│   │   │   ├── stores/        # Task state (Zustand)
│   │   │   └── index.ts       # Feature exports
│   │   ├── environments/      # Environment management
│   │   ├── preview/           # Preview functionality
│   │   ├── claude-code/       # Claude Code integration
│   │   ├── navigation/        # Navigation components
│   │   └── settings/          # Settings management
│   ├── lib/                   # Shared utilities
│   │   ├── api/               # API client, query setup
│   │   │   ├── client.ts      # Base HTTP client
│   │   │   ├── query-client.ts # React Query config
│   │   │   └── query-keys.ts   # Query key factory
│   │   ├── components/        # Shared components
│   │   │   └── ui/            # shadcn/ui components
│   │   ├── hooks/             # Shared hooks
│   │   └── utils/             # Utility functions
│   ├── routes/                # Page components (TanStack Router)
│   │   ├── repositories/      # Repository routes
│   │   ├── tasks/             # Task routes
│   │   └── settings/          # Settings route
│   ├── test-utils/            # Testing utilities
│   └── main.tsx              # App entry point
└── package.json
```

## Common Workflows

### Creating a New Feature
1. Create feature directory in `src/features/[feature-name]/`
2. Add `api/` directory with query keys and endpoints
3. Create `components/` for feature-specific UI
4. Add `hooks/` for feature-specific logic
5. Export public API from `index.ts`

### Creating a New Page
1. Create route file in `src/routes/[route-name]/`
2. Use `createFileRoute` from TanStack Router
3. Fetch data using React Query hooks
4. Handle loading/error states
5. Compose UI from feature components

### Adding API Integration
1. Define route contract in `@moatless/api-contract`
2. Create API hook in feature's `api/` directory
3. Add query key to `src/lib/api/query-keys.ts`
4. Use hook in component with proper error handling

### Styling Components
1. Use Tailwind utility classes directly
2. Follow mobile-first responsive design
3. Leverage shadcn/ui components when available
4. Keep custom CSS minimal

## Navigation Guide

### Essential Resources (Read First)
1. **[Component Patterns](resources/component-patterns.md)** - React 19 component structure, hooks, and patterns
2. **[Data Fetching](resources/data-fetching.md)** - TanStack Query integration and best practices
3. **[Routing](resources/routing.md)** - TanStack Router file-based routing

### Styling & UI
4. **[Styling](resources/styling.md)** - Tailwind v4 patterns and shadcn/ui components

### Advanced Topics
5. **[Error Handling](resources/error-handling.md)** - Error boundaries, recovery strategies
6. **[Performance](resources/performance.md)** - Optimization techniques and common pitfalls

### Testing & Examples
7. **[Testing](resources/testing.md)** - Testing tier system and best practices
8. **[Complete Examples](resources/complete-examples.md)** - Full feature implementations

## Quick Start Checklist

### Before Creating a Component
- [ ] Check if similar component exists in `src/features/` or `src/lib/components/`
- [ ] Identify if it's feature-specific or shared
- [ ] Plan props as primitives when possible
- [ ] Consider composition over configuration

### Before Fetching Data
- [ ] Check if query hook exists in feature's `api/` directory
- [ ] Verify query key exists in `src/lib/api/query-keys.ts`
- [ ] Plan error and loading states
- [ ] Consider cache invalidation strategy

### Before Adding a Route
- [ ] Determine route path and hierarchy
- [ ] Identify required data fetching
- [ ] Plan layout and component composition
- [ ] Consider mobile responsiveness

### Before Optimizing Performance
- [ ] Profile actual performance issue
- [ ] Identify unnecessary re-renders
- [ ] Check React DevTools profiler
- [ ] Apply targeted optimizations only

## Getting Help

### When Working On...
- **Components**: See [Component Patterns](resources/component-patterns.md)
- **Data Fetching**: See [Data Fetching](resources/data-fetching.md)
- **Routing**: See [Routing](resources/routing.md)
- **Styling**: See [Styling](resources/styling.md)
- **Errors**: See [Error Handling](resources/error-handling.md)
- **Performance**: See [Performance](resources/performance.md)
- **Storybook**: See [Storybook](resources/storybook.md)
- **Configuration/Settings UI**: See [Configuration Dialog](resources/configuration-dialog.md)
- **Testing**: See [Testing](resources/testing.md)
- **State**: See [State Management](resources/state-management.md)
- **Derived Values**: See [Derived State](resources/derived-state.md)
- **TypeScript**: See [TypeScript Patterns](resources/typescript-patterns.md)
