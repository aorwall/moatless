# LikeC4 Architecture Files

## Current State (Updated: 2025-11-06)

**Status**: ✅ **SUCCESS** - Architecture validates with 0 errors!

This directory contains a comprehensive, validated LikeC4 architecture specification for moatless-swarm-ui.

## Files

- `specs.c4` (65 lines) - Element kinds, relationships, and styling (simplified)
- `model.c4` (319 lines) - System architecture with components, relationships, and general views
- `backend/preview/views.c4` (263 lines) - Preview feature-specific views including dynamic sequence diagrams

## Architecture Overview

The moatless-swarm-ui system consists of:

**Actors:**
- Developer - Uses the system

**Containers:**
- React Frontend (`web_app`) - User interface with feature modules
- Hono Backend (`api_server`) - API server with domain services
- SQLite Database (`database`) - Data storage
- File Storage (`file_storage`) - File system storage

**External Systems:**
- Anthropic Claude API - AI model for code generation
- GitHub - Repository hosting and management
- Auth0 - Authentication and authorization

**Frontend Features:**
- Tasks Feature - Task management UI with chat
- Preview Feature - Live application preview
- UI Components - Shared component library

**Backend Components:**
- Task Service - Task execution lifecycle
- Agent Controller - AI agent coordination
- Auth Service - Authentication & sessions
- Task Repository - Data persistence

**Events Feature Components:**
- Event Service - Core service for event emission and caching
- Events Routes - API endpoints for event streaming (SSE)
- Events Storage - Repository for persisting events to disk
- SSE Connection Manager - Manages active SSE connections
- SSE Helpers - Utility functions for SSE protocol handling

**Workspace Feature Components:**
- Workspace Coordinator - High-level orchestrator for workspace operations
- Workspace Runtime Manager - Low-level runtime state manager
- Workspace Materializer - Executes workspace provisioning plans
- File Gateway - Facade for file operations API
- Workspace Runtime Repository - Database access for workspace snapshots
- Local Environment Driver - Local execution environment driver
- Workspace Routes - API endpoints for workspace management
- File Routes - API endpoints for file operations

**Preview Feature Components:**
- Preview Routes - API endpoints for preview control
- Preview Orchestrator - Orchestrates preview server lifecycle
- Preview Runtime Service - Manages preview server runtime
- Preview Proxy Service - Proxies requests to preview servers
- Hostname Proxy Service - Handles hostname-based routing
- Preview Server - Running preview application instance
- HTML Injector - Injects inspector script into HTML

All backend elements use the generic `component` type for simplicity.

**Views:**
- System Context - High-level system overview
- Container Architecture - Component relationships
- Preview Feature Architecture - Detailed preview feature components
- **Preview Server Startup Sequence** (Dynamic) - Shows complete flow from user action to running preview server
- **Preview Server Restart Sequence** (Dynamic) - Shows restart flow with stop and start phases

## Validation

Run validation:
```bash
bunx likec4 validate docs/architecture
```

Expected output:
```
INFO  likec4.lang version 1.43.0
INFO  likec4.lang workspace: found 3 source files
```

## What Was Fixed

The previous complex architecture with 22+ files had multiple validation issues:

1. **Tag syntax errors** - Tags placed incorrectly on separate lines
2. **Element resolution errors** - Child elements couldn't be resolved in view files
3. **Over-engineered structure** - Too much nesting and complexity

**Solution:** Simplified to a minimal, validated architecture with modular file organization:
- Global specifications and model (specs.c4, model.c4)
- Feature-specific views in dedicated files (backend/preview/views.c4)

This provides a clean foundation that can be expanded incrementally while maintaining organization.

## Next Steps

To expand this architecture (validate after each step):

1. **Add external systems** (Claude API, GitHub, Auth0, etc.)
2. **Add component-level details** using `extend` for backend/frontend containers
3. **Add views** for different stakeholder perspectives
4. **Add styling** (colors, shapes) to specification
5. **Add tags** for categorization (inside element blocks, not as separate lines)

## Lessons Learned

1. **Start Simple**: Begin with minimal valid structure, add complexity incrementally
2. **Validate Frequently**: Run validation after every change (caught errors in Step 5)
3. **Flat Hierarchies**: Keep component structures flat - avoid deeply nested hierarchies
4. **Predefined Colors**: Use LikeC4's predefined color names (blue, green, amber, etc.) not hex codes
5. **View Simplicity**: Keep views simple - reference top-level containers and their direct children
6. **Incremental Addition**: Add one feature at a time and validate immediately
7. **Generic Types**: Use generic `component` type for backend elements instead of specialized types (service, coordinator, proxy, etc.) - keeps specs.c4 simple and element names are descriptive enough
8. **Neutral Element Colors**: Remove colors from `component` and `feature` elements in specs.c4 to allow view-level styling to control appearance - prevents color clutter in sequence diagrams
9. **Dynamic Views**: Use `dynamic view` for sequence diagrams showing interaction flows - include order controls left-to-right positioning
10. **Multi-Level Hierarchy**: Explicitly include intermediate containers (e.g., `include backend.previewFeature`) to show 3+ level grouping boxes
11. **View-Level Styling**: Apply `._` notation to style all children, use opacity patterns (20% children, 15% containers) for visual hierarchy
12. **Modular Organization**: Separate feature-specific views into dedicated files (e.g., `backend/preview/views.c4`) for better organization and maintainability

## Useful Commands

```bash
# Validate syntax
bunx likec4 validate docs/architecture

# Generate diagrams
bunx likec4 build docs/architecture

# Start preview server
bunx likec4 preview docs/architecture
```

## Resources

- [LikeC4 Documentation](https://likec4.dev)
- [LikeC4 Playground](https://playground.likec4.dev)
- [Project LikeC4 Guide](../likec4-guide.md) - **Comprehensive 1,476-line guide** covering:
  - Dynamic views and sequence diagrams
  - Multi-level hierarchical views
  - View-level styling patterns
  - Common pitfalls and best practices
