# Moatless TypeScript/Fastify Backend Migration Strategy

## Executive Summary

This document outlines a comprehensive strategy for migrating the Moatless Swarm backend from Python to TypeScript with Fastify as the web framework. The strategy focuses on creating a modern monolithic application that can be easily distributed via NPX, with a clear separation between server, client, and shared code.

## Current Architecture Analysis

### Backend (Python)
The current backend is built with:
- **FastAPI** for REST endpoints and WebSocket support
- **Pydantic v2** for data validation and serialization
- **File-based storage** with optional Redis caching
- **Event streaming** for real-time updates
- **Task-based orchestration** for agent lifecycle management
- **Workspace management** supporting local and Docker environments

### Frontend (TypeScript/React)
The frontend uses:
- **React 19** with TypeScript
- **TanStack Query** for data fetching and caching
- **TanStack Router** for navigation
- **Tailwind CSS v4** for styling
- **Vite** as the build tool

## Target Architecture

### Project Structure
```
my-local-app/
├── package.json          # Main package.json with scripts
├── tsconfig.json         # Root TypeScript config
├── bin/
│   └── cli.js           # NPX entry point
├── server/
│   ├── index.ts         # Fastify server entry point
│   ├── app.ts           # Fastify app configuration
│   ├── routes/          # API routes
│   ├── services/        # Business logic
│   ├── plugins/         # Fastify plugins
│   ├── models/          # Data models
│   ├── utils/           # Server utilities
│   └── config/          # Server configuration
├── client/
│   ├── src/             # React app source
│   ├── public/          # Static assets
│   ├── index.html       # HTML entry point
│   ├── vite.config.ts   # Vite configuration
│   └── package.json     # Client dependencies
└── shared/
    ├── types/           # Shared TypeScript types
    ├── constants/       # Shared constants
    └── utils/           # Shared utilities
```

## Migration Strategy

### Phase 1: Project Structure Setup ✅

See [Project Setup Guide](./docs/migration/01-project-setup.md) for detailed instructions on:
- Initializing the project structure
- Configuring TypeScript and build tools
- Setting up development environment
- Creating NPX entry point
- Configuring code quality tools

### Phase 2: Type System Migration

See [Type System Migration Guide](./docs/migration/02-type-system.md) for detailed information on:
- Python to TypeScript type mapping
- Shared type definitions
- Naming conventions
- Type validation with Zod
- Import/export patterns

### Phase 3: Server Components Migration

See [Server Components Migration Guide](./docs/migration/03-server-components.md) for detailed information on:
- Fastify server setup and configuration
- Storage service migration (file-based JSON storage)
- Event system service with EventEmitter
- Task service and lifecycle management
- Service dependency injection pattern

### Phase 4: API Routes & WebSocket

See [API Routes Migration Guide](./docs/migration/04-api-routes.md) for detailed information on:
- Route registration system with Fastify
- Task API endpoints (CRUD operations)
- Environment and workspace API endpoints
- WebSocket implementation for real-time events
- Request/response validation with Zod schemas
- Error handling and standardization

### Phase 5: Environment Management

See [Environment Management Migration Guide](./docs/migration/05-environment-management.md) for detailed information on:
- Environment service migration from Python
- Repository configuration (local, GitHub, GitLab)
- Code execution environment setup
- File search and workspace integration
- Environment validation and templates

### Phase 6: Workspace Management

See [Workspace Management Migration Guide](./docs/migration/06-workspace-management.md) for detailed information on:
- Workspace service migration from Python
- Local workspace implementation with git worktrees
- Docker workspace support and container management
- Workspace lifecycle and cleanup utilities
- Git branch and worktree management
- Process isolation and workspace state tracking

### Phase 7: Agent Integration

See [Agent Integration Migration Guide](./docs/migration/07-agent-integration.md) for detailed information on:
- Base agent class and interface design
- Claude Code agent implementation with child processes
- Agent service for lifecycle management
- CLI process spawning and JSON line protocol parsing
- Agent timeout handling and graceful shutdown
- Agent options and configuration management

### Phase 8: Utility Functions

See [Utilities Migration Guide](./docs/migration/08-utilities.md) for detailed information on:
- Git utilities (branch management, diff parsing, worktree operations)
- File search utilities with glob patterns
- Process management utilities for CLI integration
- Async utility functions and error handling
- Path manipulation and file operations
- Performance optimization and rate limiting

### Phase 9: Service Integration

See [Service Integration Guide](./docs/migration/09-service-integration.md) for detailed information on:
- Service dependency injection pattern
- Fastify service decoration and registration
- Service lifecycle management
- Type declarations for Fastify extensions
- Service orchestration and startup sequence

### Phase 10: Client Integration & Production Build

See [Client Integration Guide](./docs/migration/10-client-integration.md) for detailed information on:
- React application setup with TanStack Router and Query
- API client integration with typed endpoints  
- Production build configuration for monolithic deployment
- Static file serving and client-server integration
- Environment variable management and deployment setup

## Master Migration Checklist

**Note:** This checklist represents the complete migration roadmap. Items can be reorganized and reprioritized as needed throughout the migration. Check off items as they are completed and add new tasks as they are discovered.

### How to Use This Checklist
1. **Keep this document open** during the entire migration process
2. **Check off items** as you complete them using `[x]`
3. **Add new tasks** to the "Additional Tasks" section as you discover them
4. **Reorganize phases** if dependencies change or better approaches are found
5. **Update time estimates** based on actual progress
6. **Add notes** directly in the checklist using comments: `- [x] Task name <!-- Completed by John, used Zod for validation -->`
7. **Review weekly** to assess progress and adjust priorities

### 🧪 TESTING REQUIREMENTS - MANDATORY FOR EACH PHASE 🧪

**IMPORTANT**: Every phase MUST include comprehensive tests before being marked as complete. Tests are not optional - they are a core requirement for phase completion.

#### Testing Standards:
1. **Unit Tests**: Every new module/service must have corresponding unit tests
2. **Integration Tests**: API endpoints and service interactions must be tested
3. **Coverage Target**: Aim for >80% code coverage on critical paths
4. **Test Location**: Tests must be co-located with source files in `__tests__` directories
5. **Test Naming**: Use descriptive test names that explain the behavior being tested

#### What to Test:
- ✅ All public methods and functions
- ✅ Error handling and edge cases
- ✅ Service interactions and dependencies
- ✅ API request/response validation
- ✅ Type safety and schema validation

### Quality Gates - Run After Each Task
After completing each task, verify:
```bash
# 1. Type checking passes
pnpm type:check

# 2. Linting passes
pnpm biome:check

# 3. Tests pass (THIS IS MANDATORY)
pnpm test

# 4. Test coverage meets targets
pnpm test:coverage

# 5. Build succeeds
pnpm build
```

### Git Workflow - Commit After Each Phase
After completing all tasks in a phase:
1. Run all quality gates one final time
2. Review all changes
3. Create a descriptive commit:
   ```bash
   git add .
   git commit -m "feat(migration): complete Phase X - [Phase Name]
   
   - List key components migrated
   - Note any significant decisions
   - Reference Python files that were ported"
   ```
4. Consider creating a git tag for major milestones:
   ```bash
   git tag -a "migration-phase-X" -m "Completed Phase X of TypeScript migration"
   ```

### Development Workflow
For each task in the checklist:
1. **Create/modify files** according to the task
2. **Run quality gates** (type:check, lint, test, build)
3. **Fix any issues** before moving to next task
4. **Update tests** to maintain coverage
5. **Mark task complete** with `[x]` in this document

For phase completion:
1. **Verify all tasks** in the phase are checked
2. **Run full test suite** across all packages
3. **Update documentation** if needed
4. **Commit with descriptive message**
5. **Tag if major milestone**

### Phase 1: Foundation & Setup (Week 1) ✅
- [x] Create project directory structure
- [x] Initialize npm project and configure package.json
- [x] Set up TypeScript configuration with strict mode
- [x] Configure Fastify server with basic plugins
- [x] Set up client with Vite and React
- [x] Configure Biome for linting and formatting with naming conventions
- [x] Set up Vitest testing framework with UI and coverage
- [x] Create NPX entry point script
- [x] Configure commit linting with commitlint
- [x] Set up Turbo for monorepo task orchestration

### Phase 2: Type System (Week 2) ✅ COMPLETED
- [x] Create `shared/types` directory structure
- [x] Port `swarm/tasks/schema.py` → `task.ts`
- [x] Port `swarm/eventstream/events.py` → `event.ts`
- [x] Port `swarm/environments/environments.py` → `environment.ts`
- [x] Port `swarm/workspaces/models.py` → `workspace.ts`
- [x] Port `swarm/workspaces/schema.py` → `workspace-schema.ts`
- [x] Create API request/response types with Zod validation
- [x] Create shared error types and constants
- [x] Add shared utilities and validation functions
- [x] Generate type documentation

### Phase 3: Server Foundation (Week 3) ✅ COMPLETED
- [x] Create Fastify server setup with logging
- [x] Configure CORS and static file serving
- [x] Set up WebSocket support
- [x] Create service architecture with dependency injection
- [x] Port `swarm/storage/base.py` → `storage.ts`
- [x] Port `swarm/storage/file.py` → file storage implementation
- [x] Implement health check endpoint
- [x] Create error handling middleware
- [x] Add request validation with Zod
- [x] Create server unit tests <!-- 45 tests passing, ~80% coverage -->

### Phase 4: Core Services (Week 4)
- [ ] Port `swarm/eventstream/event_stream.py` → `event.ts`
- [ ] Port `swarm/eventstream/file_stream.py` → file event storage
- [ ] Implement event subscription system with EventEmitter
- [ ] Port `swarm/environments/environment_manager.py` → `environment.ts`
- [ ] Create environment validation and templates
- [ ] Port `swarm/tasks/task_manager.py` → `task.ts`
- [ ] Implement task lifecycle state machine
- [ ] Add background task management
- [ ] Create service unit tests
- [ ] Add integration tests for services

### Phase 5: API Routes & WebSocket (Week 5)
- [ ] Create route registration system
- [ ] Implement task API endpoints (CRUD operations)
- [ ] Add environment API endpoints
- [ ] Create workspace API endpoints
- [ ] Implement WebSocket endpoint for real-time events
- [ ] Add request/response validation with Zod schemas
- [ ] Create error response standardization
- [ ] Add API documentation with Fastify Swagger
- [ ] Create API integration tests
- [ ] Add rate limiting and security headers

### Phase 6: Workspace Management (Week 6) ✅ COMPLETED
- [x] Port `swarm/workspaces/workspace_manager.py` → `workspace.ts`
- [x] Port `swarm/workspaces/base.py` → base workspace logic
- [x] Port `swarm/workspaces/local.py` → local workspace implementation
- [x] Implement git worktree management
- [x] Port `swarm/workspaces/docker.py` → Docker workspace support
- [x] Add workspace cleanup utilities
- [x] Create workspace state tracking
- [x] Implement workspace isolation
- [x] Add workspace unit tests
- [x] Create workspace integration tests <!-- Tests added for workspace service and git utilities -->

### Phase 7: Agent Integration (Week 7)
- [ ] Port `swarm/agents/base.py` → `base-agent.ts`
- [ ] Port `swarm/agents/claude_code.py` → `claude-code-agent.ts`
- [ ] Port `swarm/agents/options.py` → `agent-options.ts`
- [ ] Create agent service for managing agent lifecycle
- [ ] Implement CLI process spawning with child_process
- [ ] Add stdout/stderr stream parsing
- [ ] Create JSON line protocol parser
- [ ] Implement agent timeout handling
- [ ] Add agent retry logic
- [ ] Create agent unit tests
- [ ] Add Claude CLI integration tests

### Phase 8: Utilities & Helpers (Week 8)
- [ ] Port `swarm/utils/git_utils.py` → `git-utils.ts`
- [ ] Port `swarm/utils/file_search.py` → `file-search.ts`
- [ ] Port `swarm/utils/subprocess.py` → `process-utils.ts`
- [ ] Add path manipulation utilities
- [ ] Create async utility functions
- [ ] Implement rate limiting utilities
- [ ] Add logging utilities with pino
- [ ] Create error handling utilities
- [ ] Add utility unit tests
- [ ] Create performance benchmarks

### Phase 9: Client Development (Week 9)
- [ ] Set up React app with TypeScript
- [ ] Configure TanStack Query for data fetching
- [ ] Create API client with typed endpoints
- [ ] Implement task list view
- [ ] Create task detail view with real-time updates
- [ ] Add environment management UI
- [ ] Implement WebSocket connection for events
- [ ] Create responsive layout with Tailwind CSS
- [ ] Add error handling and loading states
- [ ] Create client-side routing

### Phase 10: Integration & Production Build (Week 10)
- [ ] Create production build configuration
- [ ] Set up static file serving for client
- [ ] Implement graceful shutdown handling
- [ ] Add process management for long-running tasks
- [ ] Create Docker configuration
- [ ] Add environment variable management
- [ ] Implement database migration system (if needed)
- [ ] Create backup and restore utilities
- [ ] Add monitoring and metrics endpoints
- [ ] Set up structured logging

### Phase 11: Testing & Quality (Week 11)
- [ ] Set up end-to-end testing with Playwright
- [ ] Create comprehensive integration test suite
- [ ] Add API contract tests
- [ ] Implement load testing with k6
- [ ] Create memory leak detection tests
- [ ] Add security audit with npm audit
- [ ] Set up GitHub Actions CI/CD pipeline
- [ ] Create automated npm publishing
- [ ] Add code coverage reporting
- [ ] Implement performance monitoring

### Phase 12: Documentation & Deployment (Week 12)
- [ ] Create comprehensive README
- [ ] Write API documentation with examples
- [ ] Create migration guide from Python version
- [ ] Add troubleshooting guide
- [ ] Create developer contribution guide
- [ ] Set up npm package publishing
- [ ] Create Docker Hub image
- [ ] Add installation instructions for NPX
- [ ] Create example projects
- [ ] Set up documentation website

### Additional Tasks (As Discovered)
- [ ] _Space for new tasks discovered during migration_
- [ ] _Add items here as needed_

## Technical Considerations

### 1. Architecture Decisions
- **Monolithic Design**: Single npm package containing both server and client
- **File-Based Storage**: JSON files for persistence (similar to Python version)
- **Real-time Updates**: WebSocket for event streaming
- **Process Management**: Child processes for agent execution

### 2. Error Handling
```typescript
// shared/constants/errors.ts
export class MoatlessError extends Error {
  constructor(
    message: string,
    public code: string,
    public statusCode: number = 500,
    public details?: any
  ) {
    super(message);
    this.name = 'MoatlessError';
  }
}

export class TaskNotFoundError extends MoatlessError {
  constructor(taskId: string) {
    super(`Task ${taskId} not found`, 'TASK_NOT_FOUND', 404);
  }
}

export class WorkspaceError extends MoatlessError {
  constructor(message: string, taskId: string) {
    super(message, 'WORKSPACE_ERROR', 500, { taskId });
  }
}

export class ValidationError extends MoatlessError {
  constructor(message: string, errors: any) {
    super(message, 'VALIDATION_ERROR', 400, { errors });
  }
}

// server/plugins/error-handler.ts
import { FastifyError, FastifyReply, FastifyRequest } from 'fastify';
import { MoatlessError } from '@shared/constants/errors';

export async function errorHandler(
  error: FastifyError | MoatlessError,
  request: FastifyRequest,
  reply: FastifyReply
) {
  if (error instanceof MoatlessError) {
    return reply.status(error.statusCode).send({
      error: {
        code: error.code,
        message: error.message,
        details: error.details
      }
    });
  }
  
  // Log unexpected errors
  request.log.error(error);
  
  return reply.status(500).send({
    error: {
      code: 'INTERNAL_SERVER_ERROR',
      message: 'An unexpected error occurred'
    }
  });
}
```

### 3. Type Safety
- Use Zod for runtime validation of API requests/responses
- Strict TypeScript configuration
- Shared types between server and client
- Type-safe API client generation

### 4. Performance Optimization
- Efficient file I/O with streaming for large files
- Event batching for WebSocket transmission
- Background task management with worker threads (if needed)
- Connection pooling for external services
- Response compression with @fastify/compress

### 5. Security Considerations
- CORS configuration for API endpoints
- Input validation with Zod schemas
- Secure environment variable handling
- File path sanitization to prevent directory traversal
- Process isolation for agent execution


### 6. NPX Distribution
```json
// package.json configuration for NPX
{
  "name": "moatless-app",
  "bin": {
    "moatless": "./bin/cli.js"
  },
  "files": [
    "bin/",
    "dist/",
    "client/dist/",
    "shared/"
  ],
  "engines": {
    "node": ">=18.0.0"
  }
}
```