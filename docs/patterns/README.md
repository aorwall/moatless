# Architectural Patterns for LLMs

This directory contains architectural patterns specifically written for LLMs (Language Learning Models) to understand and implement correctly when working with this codebase. Each pattern includes concrete code examples and clear implementation instructions.

## Pattern Index

### Performance Patterns
- [Tri-Context Pattern](./tri-context.md) - Optimized React context separation for performance
- [Smart Cache Invalidation](./cache-invalidation.md) - Event-driven cache management strategy

### Real-time Patterns
- [SSE Connection Management](./sse-management.md) - Server-sent events architecture

### State Management Patterns
- [React Query Patterns](./react-query.md) - Server state management best practices
- [Event-Driven Architecture](./event-driven.md) - Central event system design
- [Derived State Without Effects](./derived-state.md) - Calculate values during render instead of using effects
- [Task Lifecycle](./task-lifecycle.md) - Task, workspace, environment, and agent state management

### Error Handling Patterns
- [Error Boundary Strategies](./error-handling.md) - Graceful error recovery

### Testing Patterns
- [Strategic Mocking](./strategic-mocking.md) - Balanced testing approach

### UI & Component Patterns
- [Configuration Dialog Pattern](./configuration-dialog.md) - Reusable ChatGPT-style configuration dialogs with sidebar navigation
- [Configuration API Pattern](./configuration-api.md) - PATCH-based API design for granular configuration updates

### API & Backend Patterns
- [API Patterns and Best Practices](api-best-practices.md) - Schema-first API architecture and implementation patterns
- [Schema Design Patterns](schema-design-patterns.md) - Zod schema design principles and anti-patterns
- [Dependency Injection](./dependency-injection.md) - Service container pattern for TypeScript Hono applications
- [Docker Architecture](./docker-architecture.md) - Complete Docker-based code execution architecture and lifecycle
