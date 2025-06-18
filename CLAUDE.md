# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

### Development Commands 
```bash
# Install dependencies (requires pnpm 10.7.1)
pnpm install

# Start development server on port 3000
pnpm dev

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
- You MUST run `pnpm biome:check` after code changes
- You can run both `biome:fix` and `biome:check`

## Architecture

### Project Structure
```
src/
├── features/          # Feature-based modules
│   └── sessions/      # Sessions feature
│       ├── components/   # Session-related components
│       ├── hooks/        # Session-specific hooks
│       ├── api/          # Session API endpoints
│       └── index.ts      # Feature exports
├── lib/               # Shared utilities and components
│   ├── api/           # Core API utilities
│   │   ├── client.ts     # Base HTTP client
│   │   ├── query-client.ts # React Query setup
│   │   ├── query-keys.ts   # Query key factory
│   │   ├── sse.ts        # Server-sent events
│   │   └── websocket.ts  # WebSocket client
│   ├── components/    # Shared components
│   │   └── ui/          # Generic UI components
│   ├── layout/        # Layout components
│   ├── styles/        # Global styles
│   └── utils/         # Utility functions
├── routes/            # Page components & routing
├── types/             # TypeScript type definitions
└── main.tsx          # Application entry point
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
const { data, isLoading, error } = useSessionsList({ activeOnly: true });

// Use mutation hooks for POST/PUT/DELETE
const createSession = useCreateSession({
  onSuccess: (data) => {
    // Handle success
  },
});
```

#### Query Key Management
Query keys are centralized in `src/lib/api/query-keys.ts`:
```typescript
queryKeys.sessions() // All session queries
queryKeys.sessionDetail(id) // Specific session
queryKeys.sessionMessages(id, filters) // Session messages
```

#### WebSocket Integration
WebSocket connections automatically invalidate related queries:
```typescript
const ws = new SessionWebSocket(sessionId, queryClient);
// Messages received via WebSocket will trigger query invalidation
```

### Development Patterns

- **Path Aliases**: `@/*` maps to `./src/*` in TypeScript and Vite
- **File Naming**: Kebab-case convention enforced by Biome
- **Feature-Based Architecture**:
  - Features contain components, hooks, and API code
  - Shared utilities in `/lib/`
  - Page logic lives in `/routes/` files
- **Separation of Concerns**:
  - **Routes (Pages)**: Route handling, data fetching, page-level state, layout composition
  - **Components**: UI implementation, component state, event handlers, styling
  - Pages orchestrate, components implement
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
- **Type Safety**:
  - Define all API types in `src/types/`
  - Use TypeScript strict mode
  - Avoid `any` types
- **State Management**:
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
```