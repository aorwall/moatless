# Sandbox App: Feature-Based Architecture Refactor

## Overview

Refactor `apps/sandbox/src/` from a flat structure to a feature-based architecture matching the backend pattern. This improves maintainability, testability, and consistency across the codebase.

---

## Current Structure (Flat)

```
sandbox/src/
├── services/
│   ├── base-agent.ts
│   ├── claude-code-agent.ts
│   ├── file.service.ts
│   ├── git.service.ts
│   └── workspace-provisioner.ts
├── features/
│   └── preview/           # Only feature with structure
│       ├── services/
│       ├── types/
│       └── utils/
├── middleware/
├── utils/
├── config/
├── app.ts                 # Routes defined inline
└── index.ts
```

---

## Target Structure (Feature-Based)

```
sandbox/src/
├── features/
│   ├── agents/                    # Agent framework & execution
│   │   ├── api/
│   │   │   └── agent.routes.ts
│   │   ├── base/                  # Base agent framework
│   │   │   └── base-agent.ts
│   │   ├── claude-code/           # Claude Code agent implementation
│   │   │   ├── claude-code-agent.ts
│   │   │   └── context-tokens.ts
│   │   └── index.ts
│   │
│   ├── workspace/                 # Workspace provisioning & status
│   │   ├── api/
│   │   │   └── workspace.routes.ts
│   │   ├── services/
│   │   │   └── workspace-provisioner.ts
│   │   └── index.ts
│   │
│   ├── files/                     # File operations
│   │   ├── api/
│   │   │   └── files.routes.ts
│   │   ├── services/
│   │   │   └── file.service.ts
│   │   ├── utils/
│   │   │   └── file-tree-builder.ts
│   │   └── index.ts
│   │
│   ├── git/                       # Git operations
│   │   ├── api/
│   │   │   └── git.routes.ts
│   │   ├── services/
│   │   │   └── git.service.ts
│   │   ├── utils/
│   │   │   └── git.utils.ts
│   │   └── index.ts
│   │
│   └── preview/                   # Preview server (already exists)
│       ├── api/
│       │   └── preview.routes.ts   # NEW
│       ├── services/
│       │   └── preview-server.ts
│       ├── types/
│       ├── utils/
│       └── index.ts               # UPDATE
│
├── middleware/
│   ├── logger.ts
│   └── request-id.ts
│
├── shared/                        # Cross-feature utilities
│   └── logger.ts
│
├── config/
│   └── environment.ts
│
├── app.ts                         # Feature route composition
└── index.ts                       # Entry point
```

---

## Implementation Steps

### Step 1: Create Feature Directories

```bash
mkdir -p apps/sandbox/src/features/agents/{api,base,claude-code}
mkdir -p apps/sandbox/src/features/{workspace,files,git}/{api,services,utils}
mkdir -p apps/sandbox/src/shared
```

### Step 2: Move Files to Features

#### Agents Feature
| From | To |
|------|-----|
| `services/base-agent.ts` | `features/agents/base/base-agent.ts` |
| `services/claude-code-agent.ts` | `features/agents/claude-code/claude-code-agent.ts` |
| `utils/context-tokens.ts` | `features/agents/claude-code/context-tokens.ts` |

#### Workspace Feature
| From | To |
|------|-----|
| `services/workspace-provisioner.ts` | `features/workspace/services/workspace-provisioner.ts` |

#### Files Feature
| From | To |
|------|-----|
| `services/file.service.ts` | `features/files/services/file.service.ts` |
| `utils/file-tree-builder.ts` | `features/files/utils/file-tree-builder.ts` |

#### Git Feature
| From | To |
|------|-----|
| `services/git.service.ts` | `features/git/services/git.service.ts` |
| `utils/git-utils.ts` | `features/git/utils/git.utils.ts` |

#### Shared
| From | To |
|------|-----|
| `utils/logger.ts` | `shared/logger.ts` |

### Step 3: Create Route Files

Each feature gets a route file that exports a registration function.

#### `features/agents/api/agent.routes.ts`
```typescript
import type { Hono } from 'hono'
import { ClaudeCodeAgent } from '../claude-code/claude-code-agent'
import type { AppBindings } from '../../../app'

export function registerAgentRoutes(app: Hono<AppBindings>) {
  // POST /execute-agent - Start agent execution
  app.post('/execute-agent', async (c) => {
    const { workspaceDir, config } = c.var
    // ... implementation
  })

  // POST /stop-agent - Stop running agent
  app.post('/stop-agent', async (c) => {
    // ... implementation
  })
}
```

#### `features/workspace/api/workspace.routes.ts`
```typescript
import type { Hono } from 'hono'
import type { AppBindings } from '../../../app'

export function registerWorkspaceRoutes(app: Hono<AppBindings>) {
  // GET /workspace/status - K8s readiness probe
  app.get('/workspace/status', (c) => {
    const { provisioningResult, config } = c.var
    return c.json({
      taskId: config.taskId,
      workspaceDir: config.workspaceDir,
      status: provisioningResult.status,
      error: provisioningResult.error,
    })
  })
}
```

#### `features/files/api/files.routes.ts`
```typescript
import type { Hono } from 'hono'
import { FileService } from '../services/file.service'
import type { AppBindings } from '../../../app'

export function registerFileRoutes(app: Hono<AppBindings>) {
  const fileService = new FileService()

  app.post('/read-file', async (c) => {
    const { path } = await c.req.json()
    const { workspaceDir } = c.var
    const content = await fileService.readFile(workspaceDir, path)
    return c.json({ content })
  })

  app.post('/write-file', async (c) => {
    // ... implementation
  })

  app.post('/list-files', async (c) => {
    // ... implementation
  })

  app.post('/search-files', async (c) => {
    // ... implementation
  })

  app.post('/file-tree', async (c) => {
    // ... implementation
  })
}
```

#### `features/git/api/git.routes.ts`
```typescript
import type { Hono } from 'hono'
import { GitService } from '../services/git.service'
import type { AppBindings } from '../../../app'

export function registerGitRoutes(app: Hono<AppBindings>) {
  const gitService = new GitService()

  app.post('/git-status', async (c) => {
    const { workspaceDir } = c.var
    const status = await gitService.getStatus(workspaceDir)
    return c.json(status)
  })

  app.post('/git-changes', async (c) => {
    // ... implementation
  })

  app.post('/git-diff', async (c) => {
    // ... implementation
  })
}
```

#### `features/preview/api/preview.routes.ts`
```typescript
import type { Hono } from 'hono'
import type { AppBindings } from '../../../app'

export function registerPreviewRoutes(app: Hono<AppBindings>) {
  app.post('/preview/start', async (c) => {
    // ... implementation
  })

  app.post('/preview/stop', async (c) => {
    // ... implementation
  })

  app.get('/preview/status', async (c) => {
    // ... implementation
  })
}
```

### Step 4: Create Feature Index Files (Barrel Exports)

#### `features/agents/index.ts`
```typescript
// Routes
export { registerAgentRoutes } from './api/agent.routes'

// Base agent framework
export { BaseAgent } from './base/base-agent'
export type { AgentOptions, AgentRunOptions, FileAttachment } from './base/base-agent'

// Claude Code agent
export { ClaudeCodeAgent } from './claude-code/claude-code-agent'
export { calculateContextTokens } from './claude-code/context-tokens'
```

#### `features/workspace/index.ts`
```typescript
export { registerWorkspaceRoutes } from './api/workspace.routes'
export { provisionWorkspace } from './services/workspace-provisioner'
export type { ProvisioningResult, ProvisioningStatus } from './services/workspace-provisioner'
```

#### `features/files/index.ts`
```typescript
export { registerFileRoutes } from './api/files.routes'
export { FileService } from './services/file.service'
export { buildFileTree } from './utils/file-tree-builder'
```

#### `features/git/index.ts`
```typescript
export { registerGitRoutes } from './api/git.routes'
export { GitService } from './services/git.service'
export * from './utils/git.utils'
```

#### `features/preview/index.ts` (UPDATE)
```typescript
export { registerPreviewRoutes } from './api/preview.routes'
export { PreviewServer } from './services/preview-server'
export type { PreviewServerConfig, PreviewStatus } from './types'
```

### Step 5: Update `app.ts` - Feature Composition

```typescript
import { Hono } from 'hono'
import { requestId } from './middleware/request-id'
import { loggerMiddleware } from './middleware/logger'
import type { SandboxConfig } from './config/environment'
import type { ProvisioningResult } from './features/workspace'

// Feature route registrations
import { registerAgentRoutes } from './features/agents'
import { registerWorkspaceRoutes } from './features/workspace'
import { registerFileRoutes } from './features/files'
import { registerGitRoutes } from './features/git'
import { registerPreviewRoutes } from './features/preview'

export type AppBindings = {
  Variables: {
    requestId: string
    config: SandboxConfig
    provisioningResult: ProvisioningResult
    workspaceDir: string
  }
}

export function createApp(config: SandboxConfig, provisioningResult: ProvisioningResult) {
  const app = new Hono<AppBindings>()

  // Middleware
  app.use('*', requestId())
  app.use('*', loggerMiddleware())
  app.use('*', async (c, next) => {
    c.set('config', config)
    c.set('provisioningResult', provisioningResult)
    c.set('workspaceDir', config.workspaceDir)
    await next()
  })

  // Health check (root level)
  app.get('/health', (c) => c.json({ status: 'ok' }))

  // Register feature routes
  registerWorkspaceRoutes(app)
  registerFileRoutes(app)
  registerGitRoutes(app)
  registerAgentRoutes(app)
  registerPreviewRoutes(app)

  return app
}
```

### Step 6: Update Import Paths in Moved Files

#### `features/agents/claude-code/claude-code-agent.ts`
```typescript
// Old imports
import { BaseAgent } from './base-agent'
import { calculateContextTokens } from '../../utils/context-tokens'
import { logger } from '../../utils/logger'

// New imports
import { BaseAgent } from '../base/base-agent'
import { calculateContextTokens } from './context-tokens'
import { logger } from '../../../shared/logger'
```

#### Other files - update `../utils/logger` → `../../shared/logger`

### Step 7: Clean Up Old Directories

```bash
rm -rf apps/sandbox/src/services/
rm -rf apps/sandbox/src/utils/  # After moving logger to shared/
```

---

## Verification

```bash
# Type check
bun run type:check

# Linting
bun run biome:check

# Run sandbox in dev mode
cd apps/sandbox && bun run dev

# Test endpoints
curl http://localhost:3001/health
curl http://localhost:3001/workspace/status
```

---

## Notes

### Agent Subdirectory Pattern

The `agents/` feature uses subdirectories for different agent implementations:
- `agents/base/` - Abstract base class and shared types
- `agents/claude-code/` - Claude Code SDK agent implementation
- Future agents would get their own subdirectory (e.g., `agents/openai/`)

### No DI Container for Sandbox (Yet)

Unlike the backend, the sandbox doesn't need InversifyJS initially:
- Simpler application with fewer services
- Services can be instantiated directly in route handlers
- Can add DI later if complexity increases

### Service State Management

Some services need workspace context passed per request:
```typescript
// Option 1: Pass workspaceDir to each method
fileService.readFile(workspaceDir, path)

// Option 2: Create service instance per request
const fileService = new FileService(workspaceDir)
```

Prefer Option 1 for stateless services, Option 2 for stateful ones.

### Shared Utils Location

Cross-feature utilities go in `shared/`:
- `shared/logger.ts` - Logging utility used by all features

Feature-specific utilities stay within the feature:
- `features/files/utils/file-tree-builder.ts`
- `features/git/utils/git.utils.ts`
- `features/agents/claude-code/context-tokens.ts`
