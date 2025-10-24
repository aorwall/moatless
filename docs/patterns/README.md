# Architectural Patterns for LLMs

This directory contains architectural patterns specifically written for LLMs (Language Learning Models) to understand and implement correctly when working with this codebase. Each pattern includes concrete code examples and clear implementation instructions.

## Pattern Index

### TypeScript Patterns
- [TypeScript Syntax](./typescript-syntax.md) - Identifier matrix, control-flow, coercion, and comment rules
- [TypeScript Modules](./typescript-modules.md) - Import/export policies, decorator ordering, mutable export bans

### Performance Patterns
- [Performance Optimization](./performance-optimization.md) - useMemo, useCallback, React.memo decision rules and common pitfalls
- [Smart Cache Invalidation](./cache-invalidation.md) - Event-driven cache management strategy

### Real-time Patterns
- [SSE Connection Management](./sse-management.md) - Server-sent events architecture

### State Management Patterns
- [State Management](./state-management.md) - State co-location, lifting state, useReducer, and context splitting
- [React Query Patterns](./react-query.md) - Server state management best practices
- [Event-Driven Architecture](./event-driven.md) - Central event system design
- [Derived State Without Effects](./derived-state.md) - Calculate values during render instead of using effects
- [Task Lifecycle](./task-lifecycle.md) - Task, workspace, environment, and agent state management

### Error Handling Patterns
- [Error Boundary Strategies](./error-handling.md) - Graceful error recovery

### Testing Patterns
- [Strategic Mocking](./strategic-mocking.md) - Balanced testing approach

### UI & Component Patterns
- [Component Design Principles](./component-design.md) - Single responsibility, component sizing, prop design, and composition patterns
- [Storybook Patterns](./storybook-patterns.md) - Story placement, meta configuration, variant coverage, and interaction testing guidance
- [Configuration Dialog Pattern](./configuration-dialog.md) - Reusable ChatGPT-style configuration dialogs with sidebar navigation
- [Configuration API Pattern](./configuration-api.md) - PATCH-based API design for granular configuration updates

### API & Backend Patterns
- [API Patterns and Best Practices](api-best-practices.md) - Schema-first API architecture and implementation patterns
- [Schema Design Patterns](schema-design-patterns.md) - Zod schema design principles and anti-patterns
- [Dependency Injection](./dependency-injection.md) - Service container pattern for TypeScript Hono applications
- [Layered Architecture](./layered-architecture.md) - Explicit layer responsibilities and dependency rules for routes, services, and data layers
- [Configuration Management](./configuration-management.md) - Type-safe configuration with Zod validation and centralized env var management
- [Async Error Handling](./async-error-handling.md) - Async patterns, error handling strategies, and graceful shutdown
- [Docker Architecture](./docker-architecture.md) - Complete Docker-based code execution architecture and lifecycle
