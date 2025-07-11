# Shared Architectural Patterns

This directory contains documentation for architectural patterns used across multiple components in the Moatless Swarm UI. These patterns represent proven solutions to common challenges in our system.

## Pattern Index

### Performance Patterns
- [Tri-Context Pattern](./tri-context.md) - Optimized React context separation for performance
- [Smart Cache Invalidation](./cache-invalidation.md) - Event-driven cache management strategy

### Real-time Patterns
- [SSE Connection Management](./sse-management.md) - Server-sent events architecture
- [Optimistic UI Updates](./optimistic-updates.md) - Immediate feedback for user actions

### State Management Patterns
- [React Query Patterns](./react-query.md) - Server state management best practices
- [Event-Driven Architecture](./event-driven.md) - Central event system design
- [Derived State Without Effects](./derived-state.md) - Calculate values during render instead of using effects

### Error Handling Patterns
- [Error Boundary Strategies](./error-handling.md) - Graceful error recovery

### Testing Patterns
- [Strategic Mocking](./strategic-mocking.md) - Balanced testing approach

## When to Use These Patterns

Each pattern documentation includes:
- **Problem**: What challenge the pattern solves
- **Solution**: How the pattern addresses the problem
- **Implementation**: Code examples and usage guidelines
- **Trade-offs**: Benefits and limitations
- **Examples**: Real usage in the codebase

## Contributing New Patterns

When documenting a new pattern:
1. Ensure it's used in at least 2 different components
2. Follow the pattern documentation template
3. Include concrete examples from the codebase
4. Document both benefits and trade-offs
5. Add to this index with appropriate categorization