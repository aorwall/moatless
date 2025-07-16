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

### Error Handling Patterns
- [Error Boundary Strategies](./error-handling.md) - Graceful error recovery

### Testing Patterns
- [Strategic Mocking](./strategic-mocking.md) - Balanced testing approach

### API & Backend Patterns
- [API Patterns and Best Practices](api-best-practices.md) - Schema-first API architecture and implementation patterns
- [Schema Design Patterns](schema-design-patterns.md) - Zod schema design principles and anti-patterns
- [Environment Delegation Pattern](environment-delegation.md) - Delegating operations to environment-specific implementations
