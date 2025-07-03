# Minimal Worker Mode Implementation Guide

## Overview

This document provides a practical implementation guide for adding a minimal worker mode to the Moatless backend. Unlike the comprehensive Docker worker architecture, this approach focuses on creating a lightweight worker process that can run agents with minimal service dependencies.

## Analysis of Current Backend

### Service Dependencies

Based on the backend initialization analysis, here's the service dependency graph for agent execution:

```
AgentService
├── StorageBackend (FileStorage)
├── EventService
│   └── StorageBackend
└── WorkspaceService
    ├── RepositoryService
    │   └── StorageBackend
    ├── EventService
    └── StorageBackend
```

### Minimal Required Services

For agent execution in worker mode, we only need:
1. **StorageBackend** - Read task data, save results
2. **EventService** - Emit agent events
3. **WorkspaceService** - Manage workspace lifecycle
4. **RepositoryService** - Required by WorkspaceService
5. **AgentService** - Execute the agent

### Services NOT Required for Workers

- CommandService - Only needed for UI command management
- PreviewEnvironmentService - Only needed for dev server management
- HTTP server (Fastify) - Workers don't serve HTTP requests
- CORS, Helmet, Static file serving - Not needed for workers
- Route handlers - Workers don't expose APIs

## Implementation Plan

### 1. Create Worker Entry Point

```typescript
// apps/backend/worker.ts
import { config } from 'dotenv'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { logger } from './utils/logger.js'
import { createStorageService } from './services/storage.js'
import { createEventService } from './services/event.service.js'
import { createRepositoryService } from './services/repository.service.js'
import { createWorkspaceService } from './services/workspace.service.js'
import { createAgentService } from './services/agent.service.js'

config() // Load environment variables

const __filename = fileURLToPath(import.meta.url)
const __dirname = path.dirname(__filename)

async function startWorker() {
  const taskId = process.env.TASK_ID
  const iterationNumber = parseInt(process.env.ITERATION_NUMBER || '1', 10)
  
  if (!taskId) {
    logger.error('TASK_ID environment variable is required')
    process.exit(1)
  }

  logger.info('Starting worker', { taskId, iterationNumber })

  try {
    // Create minimal Fastify-like object for service compatibility
    const app = {
      log: logger,
      // Add any other properties services might expect
    } as any

    // Initialize only required services
    const storage = await createStorageService(app)
    const event = createEventService(app, storage)
    const repository = createRepositoryService(app, storage)
    const workspace = createWorkspaceService(app, repository, event, storage)
    
    // Set preview environment service to undefined (not needed for workers)
    workspace.setPreviewEnvironmentService(undefined as any)
    
    const agent = createAgentService(app, storage, event, workspace)

    // Get task from storage
    const task = await storage.getTask(taskId)
    if (!task) {
      throw new Error(`Task ${taskId} not found`)
    }

    // Find the iteration to execute
    const iteration = task.iterations.find(it => it.iterationNumber === iterationNumber)
    if (!iteration) {
      throw new Error(`Iteration ${iterationNumber} not found for task ${taskId}`)
    }

    logger.info('Executing agent', {
      taskId,
      iterationNumber,
      agentType: task.agentType,
      message: iteration.message
    })

    // Execute the agent
    await agent.executeAgent(
      taskId,
      iteration.message,
      iteration,
      task.agentType,
      iteration.resumeSessionId,
      task.model
    )

    logger.info('Worker completed successfully')
    process.exit(0)
  } catch (error) {
    logger.error('Worker failed', error as Error)
    process.exit(1)
  }
}

// Handle shutdown signals
process.on('SIGTERM', async () => {
  logger.info('Received SIGTERM, shutting down gracefully')
  // Agent service handles its own cleanup
  process.exit(0)
})

process.on('SIGINT', async () => {
  logger.info('Received SIGINT, shutting down gracefully')
  process.exit(0)
})

// Start the worker
startWorker()
```

### 2. Update Package.json Scripts

Add worker-specific scripts:

```json
{
  "scripts": {
    "worker": "NODE_ENV=production node dist/worker.js",
    "worker:dev": "tsx worker.ts",
    "build:worker": "tsc -p tsconfig.worker.json"
  }
}
```

### 3. Create Worker TypeScript Config

```json
// tsconfig.worker.json
{
  "extends": "./tsconfig.json",
  "compilerOptions": {
    "outDir": "./dist"
  },
  "include": [
    "worker.ts",
    "services/**/*.ts",
    "utils/**/*.ts",
    "workspaces/**/*.ts"
  ],
  "exclude": [
    "routes/**/*.ts",
    "plugins/**/*.ts",
    "app.ts",
    "index.ts"
  ]
}
```

### 4. Modify Task Service for Worker Mode

Update the task service to optionally spawn workers:

```typescript
// In task.service.ts
import { spawn } from 'node:child_process'

export class TaskService {
  private async executeAgentWithWorker(
    taskId: string,
    iteration: TaskIteration
  ): Promise<void> {
    return new Promise((resolve, reject) => {
      const env = {
        ...process.env,
        TASK_ID: taskId,
        ITERATION_NUMBER: iteration.iterationNumber.toString()
      }

      const workerProcess = spawn('node', ['dist/worker.js'], {
        env,
        stdio: 'inherit' // Forward stdout/stderr to parent
      })

      workerProcess.on('exit', (code) => {
        if (code === 0) {
          resolve()
        } else {
          reject(new Error(`Worker exited with code ${code}`))
        }
      })

      workerProcess.on('error', (error) => {
        reject(error)
      })
    })
  }

  async submitMessage(
    taskId: string,
    message: string,
    options?: SubmitMessageOptions
  ): Promise<TaskIteration> {
    // ... existing code to create iteration ...

    if (process.env.USE_WORKER_MODE === 'true') {
      // Execute in worker process
      await this.executeAgentWithWorker(taskId, iteration)
    } else {
      // Execute in-process (current behavior)
      await this.agentService.executeAgent(
        taskId,
        message,
        iteration,
        agentType,
        resumeSessionId,
        model
      )
    }

    return iteration
  }
}
```

### 5. Docker Support for Workers

Create a minimal Dockerfile for workers:

```dockerfile
# Dockerfile.worker
FROM node:20-alpine

# Install git for workspace operations
RUN apk add --no-cache git

WORKDIR /app

# Copy package files
COPY package.json pnpm-lock.yaml ./

# Install pnpm
RUN npm install -g pnpm@10.7.1

# Install dependencies
RUN pnpm install --frozen-lockfile --prod

# Copy application files
COPY dist ./dist
COPY .env.example .env

# Create data directory
RUN mkdir -p /data && chown node:node /data

# Switch to non-root user
USER node

# Set environment
ENV NODE_ENV=production
ENV DATA_DIR=/data

ENTRYPOINT ["node", "dist/worker.js"]
```

### 6. Docker Compose for Worker Testing

```yaml
# docker-compose.worker.yml
version: '3.8'

services:
  worker:
    build:
      context: ./apps/backend
      dockerfile: Dockerfile.worker
    environment:
      - NODE_ENV=production
      - TASK_ID=${TASK_ID}
      - ITERATION_NUMBER=${ITERATION_NUMBER:-1}
      - DATA_DIR=/data
      - LOG_LEVEL=info
    volumes:
      # Share data directory with main backend
      - ./data:/data
      # Repository access
      - /path/to/repos:/repos:ro
      # Worktree directory
      - /path/to/worktrees:/worktrees
    networks:
      - moatless-network

networks:
  moatless-network:
    external: true
```

### 7. Running Workers

#### Local Development

```bash
# Run worker for a specific task
TASK_ID=abc123 ITERATION_NUMBER=1 pnpm worker:dev
```

#### Docker Execution

```bash
# Build worker image
docker build -f Dockerfile.worker -t moatless-worker ./apps/backend

# Run worker
docker run --rm \
  -e TASK_ID=abc123 \
  -e ITERATION_NUMBER=1 \
  -v $(pwd)/data:/data \
  -v /repos:/repos:ro \
  -v /worktrees:/worktrees \
  moatless-worker
```

#### Programmatic Execution

```typescript
// In task service or a job queue handler
const { exec } = await import('node:child_process')
const { promisify } = await import('node:util')
const execAsync = promisify(exec)

// Using Docker
await execAsync(`docker run --rm -e TASK_ID=${taskId} -e ITERATION_NUMBER=${iteration} moatless-worker`)

// Using local Node.js
await execAsync(`TASK_ID=${taskId} ITERATION_NUMBER=${iteration} node dist/worker.js`)
```

## Event Handling in Worker Mode

Since workers share the same storage backend, events are automatically persisted and can be streamed to clients through the main backend's SSE endpoints. No additional event forwarding is needed for this minimal implementation.

## Benefits of This Approach

1. **Simplicity**: Minimal code changes, reuses existing services
2. **Compatibility**: Works with current storage and event system
3. **Flexibility**: Can run locally or in Docker
4. **Low Overhead**: No additional infrastructure required
5. **Easy Testing**: Can test workers directly without Docker

## Limitations

1. **Shared Storage**: Workers must have access to the same storage backend
2. **No Real-time Events**: Events are only visible after being written to storage
3. **Process Management**: Basic process spawning, no advanced orchestration
4. **Resource Limits**: Relies on OS/Docker for resource management

## Migration Path

### Phase 1: Local Worker Testing
1. Implement worker.ts
2. Test with USE_WORKER_MODE=true locally
3. Verify event persistence and task completion

### Phase 2: Docker Worker Testing
1. Build worker Docker image
2. Test Docker worker execution
3. Verify volume mounts and permissions

### Phase 3: Production Rollout
1. Deploy worker image to registry
2. Enable worker mode for specific tasks
3. Monitor performance and reliability

### Phase 4: Advanced Features (Optional)
1. Add job queue (BullMQ) for better orchestration
2. Implement real-time event forwarding
3. Add worker pooling for performance
4. Implement distributed tracing

## Comparison with Full Docker Worker Architecture

| Feature | Minimal Worker Mode | Full Docker Worker |
|---------|--------------------|--------------------|
| Implementation Effort | Low | High |
| Real-time Events | Via shared storage | HTTP/WebSocket forwarding |
| Process Isolation | Basic | Full container isolation |
| Resource Management | OS/Docker limits | Fine-grained control |
| Monitoring | Basic logs | Comprehensive metrics |
| Scalability | Single node | Multi-node support |
| Dependencies | None | Redis/Queue system |

## Conclusion

This minimal worker mode provides a pragmatic approach to isolating agent execution with minimal changes to the existing codebase. It's ideal for:

- Getting started with worker-based execution
- Development and testing environments
- Small to medium deployments
- Systems where shared storage is acceptable

For larger deployments requiring real-time event streaming, advanced monitoring, and multi-node scalability, consider implementing the full Docker worker architecture described in the companion document.