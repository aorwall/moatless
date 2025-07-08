# Docker Worker Mode - Required Code Changes

## Overview

This document provides specific code examples and changes needed to implement Docker Worker Mode.

## 1. Worker Mode Entry Point

### File: `apps/backend/src/worker-mode.ts` (NEW)

```typescript
import { Hono } from 'hono'
import { serve } from '@hono/node-server'
import { createLogger } from './utils/logger.js'
import { RemoteEventService } from './services/remote-event.service.js'
import { RemoteStorageBackend } from './services/remote-storage.service.js'
import { WorkspaceService } from './services/workspace.service.js'
import { AgentService } from './services/agent.service.js'
import { LocalWorkspace } from './workspaces/local.js'
import type { TaskIteration } from '@moatless/shared/types/task'

export interface WorkerModeConfig {
  apiUrl: string
  apiKey: string
  workspaceDir: string
}

export async function startWorkerMode(config: WorkerModeConfig): Promise<void> {
  const logger = createLogger('worker')
  logger.info('Starting backend in worker mode', {
    apiUrl: config.apiUrl,
    workspaceDir: config.workspaceDir
  })

  // Initialize services that proxy to main backend
  const eventService = new RemoteEventService(config.apiUrl, config.apiKey)
  const storage = new RemoteStorageBackend(config.apiUrl, config.apiKey)
  
  // Create workspace service that only uses LocalWorkspace
  const workspaceService = new WorkspaceService(storage, eventService)
  
  // Override workspace factory to always use LocalWorkspace
  workspaceService.createWorkspace = async (taskId: string, repositoryConfig: any) => {
    return new LocalWorkspace(
      taskId,
      repositoryConfig,
      eventService,
      undefined,
      undefined,
      config.workspaceDir
    )
  }
  
  // Create agent service
  const agentService = new AgentService(
    storage,
    eventService,
    workspaceService,
    null, // No repository service needed in worker mode
    null  // No environment service needed
  )
  
  // Create HTTP server
  const app = new Hono()
  
  // Health check endpoint
  app.get('/health', (c) => c.json({ 
    status: 'ok', 
    mode: 'worker',
    workspaceDir: config.workspaceDir
  }))
  
  // Agent execution endpoint
  app.post('/execute-agent', async (c) => {
    const body = await c.req.json()
    const { taskId, message, iteration, agentType, resumeSessionId, model } = body
    
    logger.info('Received agent execution request', {
      taskId,
      agentType,
      iterationNumber: iteration.iterationNumber
    })
    
    try {
      await agentService.executeAgent(
        taskId,
        message,
        iteration as TaskIteration,
        agentType || 'claude-code',
        resumeSessionId,
        model
      )
      
      logger.info('Agent execution completed', { taskId })
      return c.json({ success: true })
    } catch (error) {
      logger.error('Agent execution failed', error as Error, { taskId })
      return c.json({ 
        success: false, 
        error: error instanceof Error ? error.message : 'Unknown error' 
      }, 500)
    }
  })
  
  const port = parseInt(process.env.PORT || '8001')
  logger.info(`Worker HTTP server starting on port ${port}`)
  
  serve({
    fetch: app.fetch,
    port
  })
}
```

## 2. Remote Event Service

### File: `apps/backend/src/services/remote-event.service.ts` (NEW)

```typescript
import type { Event } from '@moatless/shared/types/event'
import type { EventService } from './event.service.js'
import { createLogger } from '../utils/logger.js'

export class RemoteEventService implements EventService {
  private logger = createLogger('remote-event')
  
  constructor(
    private apiUrl: string,
    private apiKey: string
  ) {}
  
  async emitEvent(event: Event): Promise<void> {
    try {
      const response = await fetch(`${this.apiUrl}/api/v1/worker/events`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Authorization': `Bearer ${this.apiKey}`
        },
        body: JSON.stringify(event)
      })
      
      if (!response.ok) {
        throw new Error(`Failed to forward event: ${response.statusText}`)
      }
    } catch (error) {
      this.logger.error('Failed to forward event', error as Error, {
        eventType: event.type,
        taskId: event.taskId
      })
      throw error
    }
  }
  
  // Other EventService methods that aren't needed in worker mode
  subscribeToTaskEvents(taskId: string): AsyncGenerator<Event> {
    throw new Error('Event subscription not available in worker mode')
  }
  
  getAllTaskEvents(taskId: string): Promise<Event[]> {
    throw new Error('Event retrieval not available in worker mode')
  }
  
  clearTaskEvents(taskId: string): Promise<void> {
    throw new Error('Event clearing not available in worker mode')
  }
}
```

## 3. Remote Storage Backend

### File: `apps/backend/src/services/remote-storage.service.ts` (NEW)

```typescript
import type { StorageBackend } from '../storage/types.js'
import type { Task, TaskIteration } from '@moatless/shared/types/task'
import type { RepositoryConfig } from '@moatless/shared/types/repository'
import { createLogger } from '../utils/logger.js'

export class RemoteStorageBackend implements StorageBackend {
  private logger = createLogger('remote-storage')
  
  constructor(
    private apiUrl: string,
    private apiKey: string
  ) {}
  
  private async request(path: string, options: RequestInit = {}): Promise<any> {
    const response = await fetch(`${this.apiUrl}/api/v1${path}`, {
      ...options,
      headers: {
        'Content-Type': 'application/json',
        'Authorization': `Bearer ${this.apiKey}`,
        ...options.headers
      }
    })
    
    if (!response.ok) {
      throw new Error(`Storage request failed: ${response.statusText}`)
    }
    
    return response.json()
  }
  
  async getTask(taskId: string): Promise<Task> {
    return this.request(`/tasks/${taskId}`)
  }
  
  async updateTask(taskId: string, updates: Partial<Task>): Promise<void> {
    await this.request(`/tasks/${taskId}`, {
      method: 'PATCH',
      body: JSON.stringify(updates)
    })
  }
  
  async updateIteration(taskId: string, iteration: TaskIteration): Promise<void> {
    await this.request(`/tasks/${taskId}/iterations/${iteration.iterationNumber}`, {
      method: 'PUT',
      body: JSON.stringify(iteration)
    })
  }
  
  async getRepository(repositoryId: string): Promise<RepositoryConfig> {
    return this.request(`/repositories/${repositoryId}`)
  }
  
  // Implement other StorageBackend methods similarly...
  async getTasks(): Promise<Task[]> {
    throw new Error('Task listing not available in worker mode')
  }
  
  async createTask(task: Omit<Task, 'id'>): Promise<Task> {
    throw new Error('Task creation not available in worker mode')
  }
  
  async deleteTask(taskId: string): Promise<void> {
    throw new Error('Task deletion not available in worker mode')
  }
  
  // ... other methods
}
```

## 4. Worker Service

### File: `apps/backend/src/services/worker.service.ts` (NEW)

```typescript
import Docker from 'dockerode'
import { randomUUID } from 'node:crypto'
import type { TaskIteration } from '@moatless/shared/types/task'
import { createLogger } from '../utils/logger.js'

interface WorkerInfo {
  id: string
  containerId: string
  host: string
  port: number
  apiKey: string
  status: 'starting' | 'ready' | 'busy' | 'stopping'
  tasksExecuted: number
  createdAt: Date
}

export class WorkerService {
  private docker: Docker
  private workers = new Map<string, WorkerInfo>()
  private logger = createLogger('worker-service')
  
  constructor() {
    this.docker = new Docker()
  }
  
  async executeAgentInWorker(
    taskId: string,
    message: string,
    iteration: TaskIteration,
    agentType: string,
    resumeSessionId?: string,
    model?: string,
    repositoryConfig?: any
  ): Promise<void> {
    const worker = await this.getOrCreateWorker(repositoryConfig)
    
    this.logger.info('Delegating task to worker', {
      workerId: worker.id,
      taskId,
      iterationNumber: iteration.iterationNumber
    })
    
    worker.status = 'busy'
    
    try {
      const response = await fetch(`http://${worker.host}:${worker.port}/execute-agent`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Authorization': `Bearer ${worker.apiKey}`
        },
        body: JSON.stringify({
          taskId,
          message,
          iteration,
          agentType,
          resumeSessionId,
          model
        })
      })
      
      if (!response.ok) {
        const error = await response.text()
        throw new Error(`Worker execution failed: ${error}`)
      }
      
      worker.tasksExecuted++
      worker.status = 'ready'
    } catch (error) {
      worker.status = 'ready'
      throw error
    }
  }
  
  private async getOrCreateWorker(repositoryConfig?: any): Promise<WorkerInfo> {
    // Find available worker
    for (const [id, worker] of this.workers) {
      if (worker.status === 'ready') {
        return worker
      }
    }
    
    // Create new worker if none available
    return this.createWorkerContainer(repositoryConfig)
  }
  
  private async createWorkerContainer(repositoryConfig?: any): Promise<WorkerInfo> {
    const workerId = `worker-${randomUUID().slice(0, 8)}`
    const apiKey = randomUUID()
    const port = 8001 + this.workers.size
    
    const image = repositoryConfig?.codeExecution?.workerImage || 'moatless/backend:latest'
    
    this.logger.info('Creating worker container', { workerId, image })
    
    const container = await this.docker.createContainer({
      Image: image,
      name: workerId,
      Env: [
        'WORKER_MODE=true',
        `WORKER_API_URL=http://host.docker.internal:${process.env.PORT || 8000}`,
        `WORKER_API_KEY=${apiKey}`,
        'WORKSPACE_DIR=/workspace',
        `PORT=${port}`
      ],
      ExposedPorts: {
        [`${port}/tcp`]: {}
      },
      HostConfig: {
        AutoRemove: true,
        NetworkMode: 'bridge',
        ExtraHosts: ['host.docker.internal:host-gateway'],
        PortBindings: {
          [`${port}/tcp`]: [{ HostPort: `${port}` }]
        },
        Memory: 2 * 1024 * 1024 * 1024, // 2GB
        CpuShares: 1024
      }
    })
    
    await container.start()
    
    const info: WorkerInfo = {
      id: workerId,
      containerId: container.id,
      host: 'localhost',
      port,
      apiKey,
      status: 'starting',
      tasksExecuted: 0,
      createdAt: new Date()
    }
    
    this.workers.set(workerId, info)
    
    // Wait for worker to be ready
    await this.waitForWorkerReady(info)
    
    return info
  }
  
  private async waitForWorkerReady(worker: WorkerInfo, maxAttempts = 30): Promise<void> {
    for (let i = 0; i < maxAttempts; i++) {
      try {
        const response = await fetch(`http://${worker.host}:${worker.port}/health`)
        if (response.ok) {
          worker.status = 'ready'
          this.logger.info('Worker is ready', { workerId: worker.id })
          return
        }
      } catch {
        // Worker not ready yet
      }
      
      await new Promise(resolve => setTimeout(resolve, 1000))
    }
    
    throw new Error(`Worker ${worker.id} failed to start`)
  }
  
  async stopAllWorkers(): Promise<void> {
    this.logger.info('Stopping all workers')
    
    for (const [id, worker] of this.workers) {
      try {
        const container = this.docker.getContainer(worker.containerId)
        await container.stop({ t: 10 })
      } catch (error) {
        this.logger.error('Failed to stop worker', error as Error, { workerId: id })
      }
    }
    
    this.workers.clear()
  }
}
```

## 5. Update Main Index

### File: `apps/backend/src/index.ts` (MODIFY)

Add at the beginning of the main function:

```typescript
import { startWorkerMode } from './worker-mode.js'

async function main() {
  // Check if running in worker mode
  if (process.env.WORKER_MODE === 'true') {
    const workerConfig = {
      apiUrl: process.env.WORKER_API_URL || 'http://localhost:8000',
      apiKey: process.env.WORKER_API_KEY || '',
      workspaceDir: process.env.WORKSPACE_DIR || '/workspace'
    }
    
    if (!workerConfig.apiKey) {
      throw new Error('WORKER_API_KEY is required in worker mode')
    }
    
    await startWorkerMode(workerConfig)
    return
  }
  
  // Continue with normal startup...
```

## 6. Worker Routes

### File: `apps/backend/src/routes/worker.routes.ts` (NEW)

```typescript
import type { Hono } from 'hono'
import type { AppContext } from '../types.js'

export function registerWorkerRoutes(app: Hono<AppContext>) {
  // Middleware to authenticate worker requests
  const authenticateWorker = async (c: any, next: any) => {
    const authHeader = c.req.header('Authorization')
    if (!authHeader?.startsWith('Bearer ')) {
      return c.json({ error: 'Unauthorized' }, 401)
    }
    
    const apiKey = authHeader.slice(7)
    // TODO: Validate API key against registered workers
    
    await next()
  }
  
  // Event forwarding endpoint
  app.post('/api/v1/worker/events', authenticateWorker, async (c) => {
    const event = await c.req.json()
    
    // Forward to event service
    await c.get('services').event.emitEvent(event)
    
    return c.json({ success: true })
  })
  
  // Worker registration endpoint
  app.post('/api/v1/worker/register', async (c) => {
    const { workerId, host, port } = await c.req.json()
    
    // TODO: Register worker in worker service
    
    return c.json({ 
      success: true,
      apiKey: 'generated-api-key'
    })
  })
}
```

## 7. Update Agent Service

### File: `apps/backend/src/services/agent.service.ts` (MODIFY)

Add worker delegation logic:

```typescript
export class AgentService {
  constructor(
    private storage: StorageBackend,
    private eventService: EventService,
    private workspaceService: WorkspaceService,
    private repositoryService: RepositoryService | null,
    private environmentService: EnvironmentService | null,
    private workerService?: WorkerService
  ) {}
  
  async executeAgent(
    taskId: string,
    message: string,
    iteration: TaskIteration,
    agentType: string = 'claude-code',
    resumeSessionId?: string,
    model?: string
  ): Promise<void> {
    // Get task and repository config
    const task = await this.storage.getTask(taskId)
    const repository = this.repositoryService 
      ? await this.repositoryService.getRepository(task.repositoryId)
      : await this.storage.getRepository(task.repositoryId)
    
    // Check if we should use workers
    const useWorkers = process.env.ENABLE_WORKERS === 'true' 
      && repository.codeExecution?.useWorkers 
      && this.workerService
    
    if (useWorkers) {
      // Delegate to worker
      await this.workerService.executeAgentInWorker(
        taskId,
        message,
        iteration,
        agentType,
        resumeSessionId,
        model,
        repository
      )
    } else {
      // Execute locally as before
      await this.processAgentExecution(
        taskId,
        message,
        iteration,
        agentType,
        resumeSessionId,
        model
      )
    }
  }
```

## 8. Update Repository Types

### File: `packages/shared/types/repository.ts` (MODIFY)

Add worker configuration:

```typescript
export interface CodeExecutionConfig {
  type: 'local' | 'docker'
  image?: string
  setupCommands?: string
  envVars?: Record<string, string>
  volumes?: string[]
  ports?: number[]
  
  // New worker configuration
  useWorkers?: boolean
  workerImage?: string
  workerResources?: {
    memory: string
    cpu: string
  }
}
```

## 9. Docker Compose Example

### File: `docker-compose.worker.yml` (NEW)

```yaml
version: '3.8'

services:
  backend:
    build: ./apps/backend
    ports:
      - "8000:8000"
    environment:
      - WORKER_MODE=false
      - ENABLE_WORKERS=true
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
      - ./data:/app/data
    
  # Example pre-configured worker
  worker-1:
    build: ./apps/backend
    environment:
      - WORKER_MODE=true
      - WORKER_API_URL=http://backend:8000
      - WORKER_API_KEY=dev-worker-key-123
      - WORKSPACE_DIR=/workspace
      - PORT=8001
    depends_on:
      - backend
    volumes:
      - worker1-workspace:/workspace
    
volumes:
  worker1-workspace:
```

## 10. Update Dockerfile

### File: `apps/backend/Dockerfile` (MODIFY)

Ensure git is installed for LocalWorkspace:

```dockerfile
FROM node:20-slim

WORKDIR /app

# Install git for LocalWorkspace functionality
RUN apt-get update && apt-get install -y \
    git \
    && rm -rf /var/lib/apt/lists/*

# ... rest of Dockerfile

# Create workspace directory for worker mode
RUN mkdir -p /workspace && chmod 777 /workspace

# ... rest of Dockerfile
```

## Summary

These code changes implement a Docker Worker Mode that:
1. Runs the backend in a limited "worker mode" inside containers
2. Uses `LocalWorkspace` for all agent execution
3. Forwards events back to the main backend
4. Manages worker containers automatically
5. Maintains compatibility with existing code

The implementation is designed to be incremental and backward-compatible, allowing gradual rollout and testing.