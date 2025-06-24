# Server Components Migration Guide

This guide covers the migration of core server components from Python to TypeScript with Fastify.

## Overview

Phase 3 focuses on establishing the foundational server architecture with TypeScript and Fastify, migrating the core storage, event, and task management systems from the Python codebase.

## Python Source Files to Migrate

- `/moatless-swarm/swarm/storage/base.py` - Storage interface
- `/moatless-swarm/swarm/storage/file.py` - File storage implementation
- `/moatless-swarm/swarm/tasks/task_storage.py` - Task-specific storage
- `/moatless-swarm/swarm/eventstream/event_stream.py` - Event stream management
- `/moatless-swarm/swarm/eventstream/file_stream.py` - File-based event storage
- `/moatless-swarm/swarm/tasks/task_manager.py` - Main task management logic
- `/moatless-swarm/swarm/tasks/task_storage.py` - Task storage operations
- `/moatless-swarm/swarm/tasks/response_builder.py` - Response building logic

## Implementation

### 1. Fastify Server Setup

#### server/index.ts
```typescript
import fastify from 'fastify';
import { config } from 'dotenv';
import { setupApp } from './app.js';

config(); // Load environment variables

const server = fastify({
  logger: {
    level: process.env.LOG_LEVEL || 'info',
    transport: {
      target: 'pino-pretty',
      options: {
        translateTime: 'HH:MM:ss Z',
        ignore: 'pid,hostname',
      },
    },
  },
});

const start = async () => {
  try {
    await setupApp(server);
    
    const port = parseInt(process.env.PORT || '8000', 10);
    const host = process.env.HOST || '0.0.0.0';
    
    await server.listen({ port, host });
    console.log(`Server listening on http://${host}:${port}`);
  } catch (err) {
    server.log.error(err);
    process.exit(1);
  }
};

start();
```

#### server/app.ts
```typescript
import type { FastifyInstance } from 'fastify';
import cors from '@fastify/cors';
import staticPlugin from '@fastify/static';
import websocket from '@fastify/websocket';
import helmet from '@fastify/helmet';
import path from 'path';
import { fileURLToPath } from 'url';
import { registerRoutes } from './routes/index.js';
import { setupServices } from './services/index.js';
import { errorHandler } from './plugins/error-handler.js';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

export async function setupApp(app: FastifyInstance) {
  // Security
  await app.register(helmet, {
    contentSecurityPolicy: false, // Allow inline scripts for development
  });
  
  // CORS
  await app.register(cors, {
    origin: process.env.CORS_ORIGIN || true,
    credentials: true,
  });
  
  // WebSocket support
  await app.register(websocket);
  
  // Error handling
  app.setErrorHandler(errorHandler);
  
  // Serve client build in production
  if (process.env.NODE_ENV === 'production') {
    await app.register(staticPlugin, {
      root: path.join(__dirname, '../../client/dist'),
      prefix: '/',
    });
  }
  
  // Setup services
  const services = setupServices(app);
  app.decorate('services', services);
  
  // Register API routes
  await registerRoutes(app);
  
  // Health check
  app.get('/health', async () => ({ 
    status: 'ok', 
    timestamp: new Date().toISOString(),
    version: process.env.npm_package_version || '1.0.0'
  }));
}
```

### 2. Storage Service

#### server/services/storage.ts
Port from: `swarm/storage/base.py` and `swarm/storage/file.py`

```typescript
import { promises as fs } from 'fs';
import path from 'path';
import type { FastifyInstance } from 'fastify';
import type { TaskMetadata, Environment, Workspace, Event } from '@shared/types';
import { toJSON, fromJSON } from '@shared/utils/validation.js';

export interface StorageBackend {
  // Generic storage methods
  save(key: string, data: any): Promise<void>;
  get<T = any>(key: string): Promise<T | null>;
  list(prefix?: string): Promise<string[]>;
  delete(key: string): Promise<void>;
  exists(key: string): Promise<boolean>;
  
  // Type-safe convenience methods
  saveTask(task: TaskMetadata): Promise<void>;
  getTask(taskId: string): Promise<TaskMetadata | null>;
  saveEnvironment(env: Environment): Promise<void>;
  getEnvironment(envId: string): Promise<Environment | null>;
  saveWorkspace(workspace: Workspace): Promise<void>;
  getWorkspace(taskId: string): Promise<Workspace | null>;
  saveEvents(taskId: string, events: Event[]): Promise<void>;
  getEvents(taskId: string): Promise<Event[]>;
}

export class FileStorage implements StorageBackend {
  constructor(private basePath: string) {}
  
  async save(key: string, data: any): Promise<void> {
    const filePath = await this.getPath(key);
    const dir = path.dirname(filePath);
    await fs.mkdir(dir, { recursive: true });
    await fs.writeFile(filePath, toJSON(data));
  }
  
  async get<T = any>(key: string): Promise<T | null> {
    try {
      const filePath = await this.getPath(key);
      const content = await fs.readFile(filePath, 'utf-8');
      return fromJSON<T>(content);
    } catch {
      return null;
    }
  }
  
  async list(prefix?: string): Promise<string[]> {
    const dir = path.join(this.basePath, prefix || '');
    try {
      const entries = await fs.readdir(dir, { withFileTypes: true });
      const files: string[] = [];
      
      for (const entry of entries) {
        if (entry.isFile() && entry.name.endsWith('.json')) {
          const key = prefix ? `${prefix}${entry.name.replace('.json', '')}` : entry.name.replace('.json', '');
          files.push(key);
        } else if (entry.isDirectory() && prefix) {
          // Recursively list subdirectories
          const subFiles = await this.list(`${prefix}${entry.name}/`);
          files.push(...subFiles);
        }
      }
      
      return files.sort();
    } catch {
      return [];
    }
  }
  
  async delete(key: string): Promise<void> {
    const filePath = await this.getPath(key);
    await fs.unlink(filePath);
  }
  
  async exists(key: string): Promise<boolean> {
    const filePath = await this.getPath(key);
    try {
      await fs.access(filePath);
      return true;
    } catch {
      return false;
    }
  }
  
  // Type-safe convenience methods
  async saveTask(task: TaskMetadata): Promise<void> {
    await this.save(`tasks/${task.taskId}`, task);
  }
  
  async getTask(taskId: string): Promise<TaskMetadata | null> {
    return this.get<TaskMetadata>(`tasks/${taskId}`);
  }
  
  async saveEnvironment(env: Environment): Promise<void> {
    await this.save(`environments/${env.id}`, env);
  }
  
  async getEnvironment(envId: string): Promise<Environment | null> {
    return this.get<Environment>(`environments/${envId}`);
  }
  
  async saveWorkspace(workspace: Workspace): Promise<void> {
    await this.save(`workspaces/${workspace.id}`, workspace);
  }
  
  async getWorkspace(taskId: string): Promise<Workspace | null> {
    return this.get<Workspace>(`workspaces/${taskId}`);
  }
  
  async saveEvents(taskId: string, events: Event[]): Promise<void> {
    await this.save(`events/${taskId}`, events);
  }
  
  async getEvents(taskId: string): Promise<Event[]> {
    const events = await this.get<Event[]>(`events/${taskId}`);
    return events || [];
  }
  
  private async getPath(key: string): Promise<string> {
    return path.join(this.basePath, `${key}.json`);
  }
}

export function createStorageService(app: FastifyInstance): StorageBackend {
  const dataDir = process.env.DATA_DIR || './data';
  return new FileStorage(dataDir);
}
```

### 3. Event System Service

#### server/services/event.ts
Port from: `swarm/eventstream/event_stream.py`

```typescript
import { EventEmitter } from 'events';
import { randomUUID } from 'crypto';
import type { FastifyInstance } from 'fastify';
import type { Event, EventType } from '@shared/types';
import type { StorageBackend } from './storage.js';

export type EventHandler = (event: Event) => void | Promise<void>;

export class EventService extends EventEmitter {
  constructor(private storage: StorageBackend) {
    super();
    this.setMaxListeners(100); // Increase limit for multiple subscriptions
  }
  
  async emit(event: Event): Promise<void> {
    // Ensure event has required fields
    if (!event.eventId) {
      event.eventId = randomUUID();
    }
    if (!event.timestamp) {
      event.timestamp = new Date();
    }
    
    // Store event to file (similar to file_stream.py)
    const existingEvents = await this.storage.getEvents(event.taskId);
    existingEvents.push(event);
    await this.storage.saveEvents(event.taskId, existingEvents);
    
    // Emit for real-time subscribers
    super.emit(`task:${event.taskId}`, event);
    super.emit('event', event);
    super.emit(`type:${event.type}`, event);
  }
  
  async getEvents(
    taskId: string, 
    limit?: number, 
    offset: number = 0,
    eventType?: EventType
  ): Promise<Event[]> {
    let events = await this.storage.getEvents(taskId);
    
    // Filter by event type if specified
    if (eventType) {
      events = events.filter(event => event.type === eventType);
    }
    
    // Apply pagination
    return events.slice(offset, limit ? offset + limit : undefined);
  }
  
  subscribe(taskId: string, handler: EventHandler): () => void {
    const wrappedHandler = (event: Event) => {
      Promise.resolve(handler(event)).catch(console.error);
    };
    
    this.on(`task:${taskId}`, wrappedHandler);
    
    return () => {
      this.off(`task:${taskId}`, wrappedHandler);
    };
  }
  
  subscribeAll(handler: EventHandler): () => void {
    const wrappedHandler = (event: Event) => {
      Promise.resolve(handler(event)).catch(console.error);
    };
    
    this.on('event', wrappedHandler);
    
    return () => {
      this.off('event', wrappedHandler);
    };
  }
  
  subscribeByType(eventType: EventType, handler: EventHandler): () => void {
    const wrappedHandler = (event: Event) => {
      Promise.resolve(handler(event)).catch(console.error);
    };
    
    this.on(`type:${eventType}`, wrappedHandler);
    
    return () => {
      this.off(`type:${eventType}`, wrappedHandler);
    };
  }
  
  async getEventCount(taskId: string, eventType?: EventType): Promise<number> {
    const events = await this.getEvents(taskId, undefined, 0, eventType);
    return events.length;
  }
}

export function createEventService(app: FastifyInstance, storage: StorageBackend): EventService {
  return new EventService(storage);
}
```

### 4. Task Service

#### server/services/task.ts
Port from: `swarm/tasks/task_manager.py`

```typescript
import { randomUUID } from 'crypto';
import type { FastifyInstance } from 'fastify';
import type { TaskMetadata, CreateTaskRequest, TaskIteration } from '@shared/types/task';
import type { StorageBackend } from './storage.js';
import type { EventService } from './event.js';
import type { EnvironmentService } from './environment.js';
import type { WorkspaceService } from './workspace.js';
import { TaskNotFoundError, ValidationError } from '@shared/constants/errors.js';

export class TaskService {
  private backgroundTasks = new Set<Promise<void>>();
  
  constructor(
    private storage: StorageBackend,
    private eventService: EventService,
    private environmentService: EnvironmentService,
    private workspaceService: WorkspaceService
  ) {}
  
  async createTask(params: CreateTaskRequest): Promise<string> {
    // Validate environment exists
    const environment = await this.environmentService.getEnvironment(params.environmentId);
    if (!environment) {
      throw new ValidationError(`Environment ${params.environmentId} not found`, { environmentId: params.environmentId });
    }
    
    const taskId = randomUUID();
    const now = new Date();
    
    const metadata: TaskMetadata = {
      taskId,
      createdAt: now,
      lastMessageAt: now,
      agentType: params.agentType,
      environmentId: params.environmentId,
      status: 'active',
      numTurns: 0,
      totalCostUsd: 0.0,
      iterations: []
    };
    
    await this.saveMetadata(taskId, metadata);
    
    // Emit task creation event
    await this.eventService.emit({
      eventId: randomUUID(),
      type: 'start',
      taskId,
      timestamp: now,
      isError: false,
      payload: { 
        agentType: params.agentType,
        environmentId: params.environmentId 
      }
    });
    
    // Setup workspace asynchronously in background
    const setupTask = this.setupWorkspaceAsync(
      taskId, 
      metadata, 
      params.branchName, 
      params.message, 
      params.start ?? true
    );
    
    this.backgroundTasks.add(setupTask);
    setupTask.finally(() => this.backgroundTasks.delete(setupTask));
    
    return taskId;
  }
  
  async getTask(taskId: string): Promise<TaskMetadata | null> {
    return this.storage.getTask(taskId);
  }
  
  async listTasks(limit: number = 50, offset: number = 0): Promise<TaskMetadata[]> {
    const taskKeys = await this.storage.list('tasks/');
    const tasks: TaskMetadata[] = [];
    
    for (const key of taskKeys) {
      const taskId = key.replace('tasks/', '');
      const task = await this.storage.getTask(taskId);
      if (task) tasks.push(task);
    }
    
    // Sort by last message time (most recent first)
    tasks.sort((a, b) => b.lastMessageAt.getTime() - a.lastMessageAt.getTime());
    
    // Apply pagination
    return tasks.slice(offset, offset + limit);
  }
  
  async sendMessage(taskId: string, message: string): Promise<void> {
    const metadata = await this.getTask(taskId);
    if (!metadata) {
      throw new TaskNotFoundError(taskId);
    }
    
    // Update metadata
    const now = new Date();
    const iterationNumber = metadata.iterations.length + 1;
    
    const iteration: TaskIteration = {
      iterationNumber,
      message,
      startedAt: now,
      status: 'running'
    };
    
    metadata.iterations.push(iteration);
    metadata.lastMessageAt = now;
    metadata.numTurns += 1;
    
    await this.saveMetadata(taskId, metadata);
    
    // Emit input event
    await this.eventService.emit({
      eventId: randomUUID(),
      type: 'input',
      taskId,
      iterationNumber,
      timestamp: now,
      isError: false,
      payload: { message }
    });
  }
  
  async updateTask(taskId: string, updates: Partial<TaskMetadata>): Promise<void> {
    const metadata = await this.getTask(taskId);
    if (!metadata) {
      throw new TaskNotFoundError(taskId);
    }
    
    const updatedMetadata = { ...metadata, ...updates };
    await this.saveMetadata(taskId, updatedMetadata);
  }
  
  async deleteTask(taskId: string): Promise<void> {
    const task = await this.getTask(taskId);
    if (!task) {
      throw new TaskNotFoundError(taskId);
    }
    
    // Clean up workspace
    await this.workspaceService.cleanupWorkspace(taskId);
    
    // Delete task data
    await this.storage.delete(`tasks/${taskId}`);
    await this.storage.delete(`events/${taskId}`);
    
    // Emit deletion event
    await this.eventService.emit({
      eventId: randomUUID(),
      type: 'stop',
      taskId,
      timestamp: new Date(),
      isError: false,
      payload: { reason: 'deleted' }
    });
  }
  
  private async saveMetadata(taskId: string, metadata: TaskMetadata): Promise<void> {
    await this.storage.saveTask(metadata);
  }
  
  private async setupWorkspaceAsync(
    taskId: string,
    metadata: TaskMetadata,
    branchName?: string,
    message?: string,
    start?: boolean
  ): Promise<void> {
    try {
      // Create workspace
      const workspace = await this.workspaceService.createWorkspace(
        taskId,
        metadata.environmentId,
        branchName
      );
      
      // Emit setup complete event
      await this.eventService.emit({
        eventId: randomUUID(),
        type: 'workspace',
        subType: 'setup_complete',
        taskId,
        timestamp: new Date(),
        isError: false,
        payload: { workspaceId: workspace.id }
      });
      
      if (start && message) {
        await this.sendMessage(taskId, message);
      }
    } catch (error) {
      await this.eventService.emit({
        eventId: randomUUID(),
        type: 'error',
        taskId,
        timestamp: new Date(),
        isError: true,
        payload: { 
          error: error instanceof Error ? error.message : String(error),
          phase: 'workspace_setup'
        }
      });
    }
  }
  
  async waitForBackgroundTasks(): Promise<void> {
    await Promise.all(Array.from(this.backgroundTasks));
  }
}

export function createTaskService(
  app: FastifyInstance,
  storage: StorageBackend,
  eventService: EventService,
  environmentService: EnvironmentService,
  workspaceService: WorkspaceService
): TaskService {
  return new TaskService(storage, eventService, environmentService, workspaceService);
}
```

### 5. Error Handler Plugin

#### server/plugins/error-handler.ts
```typescript
import type { FastifyError, FastifyReply, FastifyRequest } from 'fastify';
import { MoatlessError } from '@shared/constants/errors.js';

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
  
  // Handle Fastify validation errors
  if (error.validation) {
    return reply.status(400).send({
      error: {
        code: 'VALIDATION_ERROR',
        message: 'Request validation failed',
        details: error.validation
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

## Migration Checklist

- [ ] Set up Fastify server with logging and plugins
- [ ] Create storage service with file-based JSON storage
- [ ] Implement event service with EventEmitter
- [ ] Create task service with lifecycle management
- [ ] Add error handling middleware
- [ ] Create service factory functions
- [ ] Add comprehensive error types
- [ ] Implement background task management
- [ ] Add type-safe service interfaces
- [ ] Write unit tests for each service
- [ ] Add integration tests
- [ ] Validate against Python implementation behavior

## Testing Strategy

### Unit Tests
```typescript
// server/services/__tests__/storage.test.ts
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { FileStorage } from '../storage.js';
import { promises as fs } from 'fs';
import path from 'path';

describe('FileStorage', () => {
  let storage: FileStorage;
  let tempDir: string;
  
  beforeEach(async () => {
    tempDir = path.join(process.cwd(), 'temp-test-' + Date.now());
    await fs.mkdir(tempDir, { recursive: true });
    storage = new FileStorage(tempDir);
  });
  
  afterEach(async () => {
    await fs.rm(tempDir, { recursive: true, force: true });
  });
  
  it('should save and retrieve data', async () => {
    const testData = { id: '123', name: 'test' };
    await storage.save('test/item', testData);
    
    const retrieved = await storage.get('test/item');
    expect(retrieved).toEqual(testData);
  });
  
  it('should handle dates correctly', async () => {
    const testData = { timestamp: new Date() };
    await storage.save('test/date', testData);
    
    const retrieved = await storage.get('test/date');
    expect(retrieved.timestamp).toBeInstanceOf(Date);
    expect(retrieved.timestamp.getTime()).toBe(testData.timestamp.getTime());
  });
});
```

## Next Steps

1. Review [API Routes Migration](./04-api-routes.md) for endpoint implementation
2. Implement comprehensive error handling
3. Add monitoring and metrics collection
4. Set up automated testing pipeline
5. Validate storage performance with large datasets