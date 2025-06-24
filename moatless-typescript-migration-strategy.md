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

### Phase 1: Project Structure Setup

#### 1.1 Initialize Project
```bash
mkdir my-local-app && cd my-local-app
pnpm init
```

#### 1.2 Root package.json Configuration
```json
{
  "name": "my-local-app",
  "version": "1.0.0",
  "description": "Moatless TypeScript/Fastify Application",
  "bin": {
    "my-local-app": "./bin/cli.js"
  },
  "packageManager": "pnpm@10.7.1",
  "engines": {
    "node": ">=22.16.x"
  },
  "scripts": {
    "dev": "concurrently \"pnpm run dev:server\" \"pnpm run dev:client\"",
    "dev:server": "tsx watch server/index.ts",
    "dev:client": "cd client && pnpm run dev",
    "build": "pnpm run build:server && pnpm run build:client",
    "build:server": "tsc -p server/tsconfig.json",
    "build:client": "cd client && pnpm run build",
    "start": "node dist/server/index.js",
    "test": "vitest run",
    "test:watch": "vitest",
    "test:ui": "vitest --ui --coverage",
    "test:coverage": "vitest --coverage run",
    "biome:check": "biome check",
    "biome:fix": "pnpm biome:check --write",
    "biome:ci": "biome ci",
    "type:check": "tsc --noEmit",
    "turbo": "pnpm dlx turbo@2.5.0",
    "check:turbo": "pnpm turbo biome:check type:check test",
    "prepare": "husky",
    "release": "cross-env HUSKY=0 commit-and-tag-version"
  },
  "dependencies": {
    "fastify": "^4.26.0",
    "@fastify/cors": "^8.5.0",
    "@fastify/static": "^7.0.0",
    "@fastify/websocket": "^8.3.0",
    "zod": "^3.22.0",
    "pino": "^8.18.0",
    "dotenv": "^16.4.0"
  },
  "devDependencies": {
    "@types/node": "^20.11.0",
    "typescript": "^5.3.0",
    "tsx": "^4.7.0",
    "concurrently": "^8.2.0",
    "vitest": "^3.2.3",
    "@biomejs/biome": "^2.0.0",
    "husky": "^9.1.7",
    "lint-staged": "^16.1.0",
    "commit-and-tag-version": "^12.5.1",
    "@commitlint/cli": "^19.8.1",
    "@commitlint/config-conventional": "^19.8.1"
  }
}
```

#### 1.3 TypeScript Configuration
```typescript
// tsconfig.json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "lib": ["ES2022"],
    "outDir": "./dist",
    "rootDir": "./",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "forceConsistentCasingInFileNames": true,
    "resolveJsonModule": true,
    "declaration": true,
    "declarationMap": true,
    "sourceMap": true,
    "paths": {
      "@shared/*": ["./shared/*"],
      "@server/*": ["./server/*"]
    }
  },
  "include": ["server/**/*", "shared/**/*", "bin/**/*"],
  "exclude": ["node_modules", "dist", "client"]
}
```

#### 1.4 Biome Configuration
```json
// biome.json
{
  "$schema": "https://biomejs.dev/schemas/1.5.0/schema.json",
  "organizeImports": {
    "enabled": true
  },
  "linter": {
    "enabled": true,
    "rules": {
      "recommended": true,
      "style": {
        "useConst": "error",
        "useTemplate": "error",
        "noVar": "error"
      },
      "correctness": {
        "noUnusedVariables": "error",
        "noUndeclaredVariables": "error"
      },
      "complexity": {
        "noBannedTypes": "error"
      },
      "naming": {
        "useNamingConvention": {
          "level": "error",
          "options": {
            "strictCase": false,
            "conventions": [
              {
                "selector": {
                  "kind": "function"
                },
                "formats": ["camelCase"]
              },
              {
                "selector": {
                  "kind": "variable"
                },
                "formats": ["camelCase", "CONSTANT_CASE"]
              },
              {
                "selector": {
                  "kind": "typeLike"
                },
                "formats": ["PascalCase"]
              }
            ]
          }
        }
      }
    }
  },
  "formatter": {
    "enabled": true,
    "formatWithErrors": false,
    "indentStyle": "space",
    "indentWidth": 2,
    "lineWidth": 100
  },
  "javascript": {
    "formatter": {
      "quoteStyle": "single",
      "trailingComma": "es5",
      "semicolons": "always"
    }
  },
  "files": {
    "ignore": [
      "node_modules",
      "dist",
      "client/dist",
      "*.min.js",
      "coverage"
    ]
  }
}
```

#### 1.5 Git Hooks Configuration
```json
// .husky/pre-commit
#!/usr/bin/env sh
. "$(dirname -- "$0")/_/husky.sh"

pnpm lint-staged

// lint-staged.config.js
export default {
  '*.{ts,tsx,js,jsx}': ['pnpm biome:check --no-errors-on-unmatched --files-ignore-unknown=true'],
  '*.{json,md,yml,yaml}': ['pnpm biome:format --no-errors-on-unmatched --files-ignore-unknown=true']
};

// commitlint.config.js
export default {
  extends: ['@commitlint/config-conventional']
};
```

#### 1.6 CLI Entry Point
```javascript
// bin/cli.js
#!/usr/bin/env node

const { spawn } = require('child_process');
const path = require('path');

// Start the server
const serverPath = path.join(__dirname, '..', 'dist', 'server', 'index.js');
const server = spawn('node', [serverPath], {
  stdio: 'inherit',
  env: { ...process.env, NODE_ENV: 'production' }
});

server.on('error', (err) => {
  console.error('Failed to start server:', err);
  process.exit(1);
});

server.on('exit', (code) => {
  process.exit(code || 0);
});

// Handle process termination
process.on('SIGINT', () => {
  server.kill('SIGINT');
});

process.on('SIGTERM', () => {
  server.kill('SIGTERM');
});
```

#### 1.7 Mandatory Workflow
- You MUST run `pnpm type:check` after code changes to catch TypeScript errors
- You MUST run `pnpm biome:check` after code changes for linting and formatting
- You can run both `biome:fix` and `biome:check`
- For comprehensive validation, run `pnpm check:turbo` which includes all checks

### Phase 2: Type System Migration

#### 2.1 Shared Types

**Python source files to migrate:**
- `/moatless-swarm/swarm/tasks/schema.py` - Task-related types
- `/moatless-swarm/swarm/eventstream/events.py` - Event types
- `/moatless-swarm/swarm/environments/environments.py` - Environment types
- `/moatless-swarm/swarm/workspaces/schema.py` - Workspace types
- `/moatless-swarm/swarm/workspaces/models.py` - Workspace models

Create shared type definitions in the `shared/types` directory:

#### Naming Conventions

**File Naming:**
- Use kebab-case for all files: `task-manager.ts`, `event-stream.ts`, `git-utils.ts`
- Exception: React components can use PascalCase: `TaskViewer.tsx` or kebab-case: `task-viewer.tsx`
- Test files use dot notation: `task-manager.test.ts`
- Index files should export public API: `index.ts`
- No redundant suffixes when context is clear from folder (e.g., `types/task.ts` not `types/task.types.ts`)

**Directory Structure:**
```
shared/
├── types/
│   ├── index.ts              # Public exports
│   ├── task.ts               # Task-related types
│   ├── event.ts              # Event types
│   ├── environment.ts        # Environment types
│   ├── workspace.ts          # Workspace types
│   └── api.ts                # API request/response types
├── constants/
│   ├── index.ts
│   └── errors.ts
└── utils/
    ├── index.ts
    └── validation.ts

server/
├── routes/
│   ├── index.ts              # Route registration
│   ├── tasks.ts              # Task endpoints
│   ├── environments.ts       # Environment endpoints
│   └── workspaces.ts         # Workspace endpoints
├── services/
│   ├── task.ts               # Task business logic
│   ├── environment.ts        # Environment logic
│   ├── workspace.ts          # Workspace logic
│   └── storage.ts            # Storage abstraction
├── models/
│   ├── task.ts               # Task data model
│   ├── environment.ts        # Environment model
│   └── workspace.ts          # Workspace model
└── plugins/
    ├── cors.ts               # CORS configuration
    ├── static.ts             # Static file serving
    └── websocket.ts          # WebSocket support
```

**Code Naming:**
- Classes: PascalCase - `TaskManager`, `LocalWorkspace`
- Interfaces/Types: PascalCase without 'I' prefix - `TaskMetadata`, `Environment`
- Enums: PascalCase - `EventType`, `WorkspaceStatus`
- Functions: camelCase - `createTask`, `searchFiles`
- Constants: UPPER_SNAKE_CASE - `MAX_RETRY_COUNT`, `DEFAULT_TIMEOUT`
- Private members: prefix with # for ES2022 private fields or underscore - `#backgroundTasks`, `_stopRequested`
- Boolean variables: use prefixes - `isActive`, `hasError`, `shouldRetry`

**Import/Export Conventions:**
```typescript
// Use named exports for better tree-shaking
export { TaskManager } from './task-manager';
export type { TaskMetadata } from './task';

// Avoid default exports except for React components
// Bad: export default TaskManager;
// Good: export { TaskManager };

// Group imports by type
import { shell, fs } from '@tauri-apps/api';  // External
import { StorageBackend } from '@server/services/storage';  // Internal packages
import { validateTaskId } from './utils/validation';  // Local
```

```typescript
// shared/types/task.ts
// Port from: swarm/tasks/schema.py
export interface TaskMetadata {
  taskId: string;
  createdAt: Date;
  lastMessageAt: Date;
  agentType: string;
  environmentId: string;
  status: 'active' | 'completed' | 'error' | 'closed';
  numTurns: number;
  totalCostUsd: number;
  iterations: TaskIteration[];
}

export interface TaskIteration {
  iterationNumber: number;
  message: string;
  startedAt: Date;
  completedAt?: Date;
  status: 'running' | 'completed' | 'error';
  response?: string;
}

// shared/types/event.ts
// Port from: swarm/eventstream/events.py
export enum EventType {
  INPUT = 'input',
  MESSAGE = 'message',
  START = 'start',
  STOP = 'stop',
  RESULT = 'result',
  ERROR = 'error',
  WORKSPACE = 'workspace',
  SERVER = 'server'
}

export interface Event {
  eventId: string;
  type: EventType;
  subType?: string;
  taskId: string;
  iterationNumber?: number;
  timestamp: Date;
  isError: boolean;
  contextTokens?: number;
  numTurns?: number;
  totalCost?: number;
  payload: Record<string, any>;
}

// shared/types/workspace.ts
// Port from: swarm/workspaces/models.py
export interface BaseWorkspace {
  id: string;
  type: 'local' | 'docker';
  environmentId: string;
  status: 'initializing' | 'ready' | 'error';
  createdAt: Date;
  error?: string;
}

export interface LocalWorkspace extends BaseWorkspace {
  type: 'local';
  path: string;
  branch?: string;
  originalBranch?: string;
}

export interface DockerWorkspace extends BaseWorkspace {
  type: 'docker';
  containerId: string;
  port?: number;
}

export type Workspace = LocalWorkspace | DockerWorkspace;

// shared/types/api.ts
// API request/response types with Zod validation
import { z } from 'zod';

export const CreateTaskRequestSchema = z.object({
  agentType: z.string(),
  environmentId: z.string(),
  message: z.string(),
  branchName: z.string().optional(),
  start: z.boolean().default(true)
});

export type CreateTaskRequest = z.infer<typeof CreateTaskRequestSchema>;

export const TaskResponseSchema = z.object({
  taskId: z.string(),
  status: z.enum(['active', 'completed', 'error', 'closed']),
  message: z.string().optional()
});

export type TaskResponse = z.infer<typeof TaskResponseSchema>;
```

#### 2.2 Python-TypeScript Type Mapping
| Python Type | TypeScript Equivalent |
|-------------|---------------------|
| `str` | `string` |
| `int`, `float` | `number` |
| `bool` | `boolean` |
| `Dict[str, Any]` | `Record<string, any>` |
| `List[T]` | `Array<T>` |
| `Optional[T]` | `T \| null \| undefined` |
| `Union[A, B]` | `A \| B` |
| `datetime` | `Date` |
| `UUID` | `string` (with validation) |
| `Enum` | `enum` or union types |
| Pydantic Models | TypeScript interfaces |

### Phase 3: Server Components Migration

#### 3.1 Fastify Server Setup

```typescript
// server/index.ts
import fastify from 'fastify';
import { config } from 'dotenv';
import { setupApp } from './app';

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
    
    const port = parseInt(process.env.PORT || '3000', 10);
    const host = process.env.HOST || '0.0.0.0';
    
    await server.listen({ port, host });
    console.log(`Server listening on http://${host}:${port}`);
  } catch (err) {
    server.log.error(err);
    process.exit(1);
  }
};

start();

// server/app.ts
import { FastifyInstance } from 'fastify';
import cors from '@fastify/cors';
import staticPlugin from '@fastify/static';
import websocket from '@fastify/websocket';
import path from 'path';
import { registerRoutes } from './routes';
import { setupServices } from './services';

export async function setupApp(app: FastifyInstance) {
  // Register plugins
  await app.register(cors, {
    origin: process.env.CORS_ORIGIN || true,
  });
  
  await app.register(websocket);
  
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
  
  // Register routes
  await registerRoutes(app);
  
  // Health check
  app.get('/health', async () => ({ status: 'ok' }));
}
```

#### 3.2 Storage Service

**Python source files to migrate:**
- `/moatless-swarm/swarm/storage/base.py` - Storage interface
- `/moatless-swarm/swarm/storage/file.py` - File storage implementation
- `/moatless-swarm/swarm/tasks/task_storage.py` - Task-specific storage

```typescript
// server/services/storage.ts
// Port from: swarm/storage/base.py and swarm/storage/file.py
import { promises as fs } from 'fs';
import path from 'path';
import { FastifyInstance } from 'fastify';
import { TaskMetadata, Environment, Workspace, Event } from '@shared/types';

export interface StorageBackend {
  // Generic storage methods
  save(key: string, data: any): Promise<void>;
  get(key: string): Promise<any | null>;
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
    await fs.writeFile(filePath, JSON.stringify(data, null, 2));
  }
  
  async get(key: string): Promise<any | null> {
    try {
      const filePath = await this.getPath(key);
      const content = await fs.readFile(filePath, 'utf-8');
      return JSON.parse(content, this.dateReviver);
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
          files.push(entry.name.replace('.json', ''));
        }
      }
      
      return files;
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
    return this.get(`tasks/${taskId}`);
  }
  
  async saveEnvironment(env: Environment): Promise<void> {
    await this.save(`environments/${env.id}`, env);
  }
  
  async getEnvironment(envId: string): Promise<Environment | null> {
    return this.get(`environments/${envId}`);
  }
  
  async saveWorkspace(workspace: Workspace): Promise<void> {
    await this.save(`workspaces/${workspace.id}`, workspace);
  }
  
  async getWorkspace(taskId: string): Promise<Workspace | null> {
    return this.get(`workspaces/${taskId}`);
  }
  
  async saveEvents(taskId: string, events: Event[]): Promise<void> {
    await this.save(`events/${taskId}`, events);
  }
  
  async getEvents(taskId: string): Promise<Event[]> {
    const events = await this.get(`events/${taskId}`);
    return events || [];
  }
  
  private async getPath(key: string): Promise<string> {
    return path.join(this.basePath, `${key}.json`);
  }
  
  // Helper to properly deserialize dates from JSON
  private dateReviver(key: string, value: any): any {
    if (typeof value === 'string' && /\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}/.test(value)) {
      return new Date(value);
    }
    return value;
  }
}

export function createStorageService(app: FastifyInstance): StorageBackend {
  const dataDir = process.env.DATA_DIR || './data';
  return new FileStorage(dataDir);
}
```

#### 3.3 Event System Service

**Python source files to migrate:**
- `/moatless-swarm/swarm/eventstream/event_stream.py` - Event stream management
- `/moatless-swarm/swarm/eventstream/file_stream.py` - File-based event storage
- `/moatless-swarm/swarm/eventstream/events.py` - Event types and models

```typescript
// server/services/event.ts
// Port from: swarm/eventstream/event_stream.py
import { EventEmitter } from 'events';
import { FastifyInstance } from 'fastify';
import { Event, EventType } from '@shared/types';
import { StorageBackend } from './storage';

export type EventHandler = (event: Event) => void | Promise<void>;

export class EventService extends EventEmitter {
  constructor(private storage: StorageBackend) {
    super();
  }
  
  async emit(event: Event): Promise<void> {
    // Store event to file (similar to file_stream.py)
    const existingEvents = await this.storage.getEvents(event.taskId);
    existingEvents.push(event);
    await this.storage.saveEvents(event.taskId, existingEvents);
    
    // Emit for real-time subscribers
    super.emit(`task:${event.taskId}`, event);
    super.emit('event', event);
  }
  
  async getEvents(taskId: string, limit?: number, offset: number = 0): Promise<Event[]> {
    const events = await this.storage.getEvents(taskId);
    return events.slice(offset, limit ? offset + limit : undefined);
  }
  
  subscribe(taskId: string, handler: EventHandler): () => void {
    const wrappedHandler = (event: Event) => {
      handler(event).catch(console.error);
    };
    
    this.on(`task:${taskId}`, wrappedHandler);
    
    return () => {
      this.off(`task:${taskId}`, wrappedHandler);
    };
  }
  
  subscribeAll(handler: EventHandler): () => void {
    const wrappedHandler = (event: Event) => {
      handler(event).catch(console.error);
    };
    
    this.on('event', wrappedHandler);
    
    return () => {
      this.off('event', wrappedHandler);
    };
  }
}

export function createEventService(app: FastifyInstance, storage: StorageBackend): EventService {
  return new EventService(storage);
}
```

#### 3.4 Task Service

**Python source files to migrate:**
- `/moatless-swarm/swarm/tasks/task_manager.py` - Main task management logic
- `/moatless-swarm/swarm/tasks/task_storage.py` - Task storage operations
- `/moatless-swarm/swarm/tasks/response_builder.py` - Response building logic

```typescript
// server/services/task.ts
// Port from: swarm/tasks/task_manager.py
import { randomUUID } from 'crypto';
import { FastifyInstance } from 'fastify';
import { TaskMetadata, CreateTaskRequest } from '@shared/types/task';
import { StorageBackend } from './storage';
import { EventService } from './event';
import { EnvironmentService } from './environment';
import { WorkspaceService } from './workspace';

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
      throw new Error(`Environment ${params.environmentId} not found`);
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
    return this.storage.get(`tasks/${taskId}`);
  }
  
  async listTasks(): Promise<TaskMetadata[]> {
    const taskIds = await this.storage.list('tasks/');
    const tasks: TaskMetadata[] = [];
    
    for (const id of taskIds) {
      const task = await this.storage.get(`tasks/${id}`);
      if (task) tasks.push(task);
    }
    
    return tasks.sort((a, b) => 
      b.lastMessageAt.getTime() - a.lastMessageAt.getTime()
    );
  }
  
  async sendMessage(taskId: string, message: string): Promise<string> {
    const metadata = await this.getTask(taskId);
    if (!metadata) {
      throw new Error(`Task ${taskId} not found`);
    }
    
    // Emit input event
    await this.eventService.emit({
      eventId: randomUUID(),
      type: 'input',
      taskId,
      timestamp: new Date(),
      isError: false,
      payload: { message }
    });
    
    // Continue with agent execution...
    return 'Message sent';
  }
  
  private async saveMetadata(taskId: string, metadata: TaskMetadata): Promise<void> {
    await this.storage.save(`tasks/${taskId}`, metadata);
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
        payload: { error: error.message }
      });
    }
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

### Phase 4: API Routes

```typescript
// server/routes/index.ts
import { FastifyInstance } from 'fastify';
import { registerTaskRoutes } from './tasks';
import { registerEnvironmentRoutes } from './environments';
import { registerWorkspaceRoutes } from './workspaces';

export async function registerRoutes(app: FastifyInstance) {
  await app.register(registerTaskRoutes, { prefix: '/api/tasks' });
  await app.register(registerEnvironmentRoutes, { prefix: '/api/environments' });
  await app.register(registerWorkspaceRoutes, { prefix: '/api/workspaces' });
}

// server/routes/tasks.ts
import { FastifyInstance } from 'fastify';
import { CreateTaskRequestSchema, TaskResponseSchema } from '@shared/types/api';

export async function registerTaskRoutes(app: FastifyInstance) {
  const { taskService, eventService } = app.services;
  
  // Create a new task
  app.post('/', {
    schema: {
      body: CreateTaskRequestSchema,
      response: {
        200: TaskResponseSchema
      }
    }
  }, async (request, reply) => {
    const taskId = await taskService.createTask(request.body);
    return { taskId, status: 'active' };
  });
  
  // Get task by ID
  app.get('/:taskId', async (request, reply) => {
    const { taskId } = request.params as { taskId: string };
    const task = await taskService.getTask(taskId);
    
    if (!task) {
      return reply.code(404).send({ error: 'Task not found' });
    }
    
    return task;
  });
  
  // List all tasks
  app.get('/', async (request, reply) => {
    return taskService.listTasks();
  });
  
  // Send message to task
  app.post('/:taskId/messages', async (request, reply) => {
    const { taskId } = request.params as { taskId: string };
    const { message } = request.body as { message: string };
    
    await taskService.sendMessage(taskId, message);
    return { status: 'sent' };
  });
  
  // Get task events
  app.get('/:taskId/events', async (request, reply) => {
    const { taskId } = request.params as { taskId: string };
    const { limit, offset } = request.query as { limit?: string; offset?: string };
    
    const events = await eventService.getEvents(
      taskId,
      limit ? parseInt(limit) : undefined,
      offset ? parseInt(offset) : 0
    );
    
    return events;
  });
  
  // WebSocket for real-time events
  app.get('/:taskId/events/stream', { websocket: true }, (connection, request) => {
    const { taskId } = request.params as { taskId: string };
    
    const unsubscribe = eventService.subscribe(taskId, (event) => {
      connection.socket.send(JSON.stringify(event));
    });
    
    connection.socket.on('close', () => {
      unsubscribe();
    });
  });
}
```

### Phase 5: Environment Management

**Python source files to migrate:**
- `/moatless-swarm/swarm/environments/environment_manager.py` - Environment management
- `/moatless-swarm/swarm/environments/environments.py` - Environment models

```typescript
// server/services/environment.ts
// Port from: swarm/environments/environment_manager.py
import { IStorageBackend } from '@moatless/storage';
import { Environment, LocalRepository, FileSearchRequest, FileSearchResponse } from '@moatless/types';

export class EnvironmentService {
  constructor(private storage: StorageBackend) {}
  
  async saveEnvironment(env: Environment): Promise<void> {
    await this.storage.save(`environments/${env.id}`, env);
  }
  
  async getEnvironment(envId: string): Promise<Environment | null> {
    return await this.storage.get(`environments/${envId}`);
  }
  
  async listEnvironments(): Promise<Environment[]> {
    const keys = await this.storage.list('environments/');
    const environments: Environment[] = [];
    
    for (const key of keys) {
      const env = await this.storage.get(`environments/${key}`);
      if (env) environments.push(env);
    }
    
    return environments;
  }
}

export function createEnvironmentService(app: FastifyInstance, storage: StorageBackend): EnvironmentService {
  return new EnvironmentService(storage);
}
```

### Phase 6: Workspace Management

**Python source files to migrate:**
- `/moatless-swarm/swarm/workspaces/workspace_manager.py` - Workspace management
- `/moatless-swarm/swarm/workspaces/base.py` - Base workspace class
- `/moatless-swarm/swarm/workspaces/local.py` - Local workspace implementation
- `/moatless-swarm/swarm/workspaces/docker.py` - Docker workspace implementation
- `/moatless-swarm/swarm/workspaces/server.py` - Server management

```typescript
// server/services/workspace.ts
// Port from: swarm/workspaces/workspace_manager.py
import { exec } from 'child_process';
import { promisify } from 'util';
import path from 'path';
import { FastifyInstance } from 'fastify';
import { Workspace, Environment } from '@shared/types';
import { StorageBackend } from './storage';
import { EnvironmentService } from './environment';
import { EventService } from './event';
import { LocalWorkspaceImpl } from './workspaces/local';
import { DockerWorkspaceImpl } from './workspaces/docker';

const execAsync = promisify(exec);

export class WorkspaceService {
  private activeWorkspaces: Map<string, Workspace> = new Map();
  
  constructor(
    private storage: StorageBackend,
    private environmentService: EnvironmentService,
    private eventService: EventService
  ) {}
  
  async createWorkspace(
    taskId: string,
    environmentId: string,
    branchName?: string
  ): Promise<Workspace> {
    // Check if workspace already exists
    const existing = this.activeWorkspaces.get(taskId);
    if (existing) return existing;
    
    // Get environment
    const environment = await this.environmentService.getEnvironment(environmentId);
    if (!environment) {
      throw new Error(`Environment not found: ${environmentId}`);
    }
    
    // Create workspace based on code execution type
    const workspace = await this.createWorkspaceForType(
      taskId,
      environment,
      branchName
    );
    
    this.activeWorkspaces.set(taskId, workspace);
    await this.storage.save(`workspaces/${taskId}`, workspace);
    
    return workspace;
  }
  
  async getWorkspace(taskId: string): Promise<Workspace | null> {
    // Check active workspaces first
    const active = this.activeWorkspaces.get(taskId);
    if (active) return active;
    
    // Load from storage
    return this.storage.get(`workspaces/${taskId}`);
  }
  
  private async createWorkspaceForType(
    taskId: string,
    environment: Environment,
    branchName?: string
  ): Promise<Workspace> {
    if (environment.codeExecution.type === 'docker') {
      return this.createDockerWorkspace(taskId, environment);
    } else {
      return this.createLocalWorkspace(taskId, environment, branchName);
    }
  }
  
  private async createLocalWorkspace(
    taskId: string,
    environment: Environment,
    branchName?: string
  ): Promise<Workspace> {
    await this.emitSetupEvent(taskId, 'setup_started');
    
    const workspace: Workspace = {
      id: taskId,
      type: 'local',
      environmentId: environment.id,
      status: 'initializing',
      createdAt: new Date(),
      path: ''
    };
    
    if (environment.repository.type === 'local') {
      // Create git worktree
      const worktreePath = path.join(
        process.env.WORKTREE_DIR || './worktrees',
        taskId
      );
      
      try {
        const branch = branchName || `task-${taskId}`;
        await execAsync(
          `git worktree add "${worktreePath}" -b "${branch}"`,
          { cwd: environment.repository.path }
        );
        
        workspace.path = worktreePath;
        workspace.branch = branch;
        workspace.status = 'ready';
      } catch (error) {
        workspace.status = 'error';
        workspace.error = error.message;
        throw error;
      }
    }
    
    await this.emitSetupEvent(taskId, 'setup_completed');
    return workspace;
  }
  
  private async createDockerWorkspace(
    taskId: string,
    environment: Environment
  ): Promise<Workspace> {
    // Docker workspace implementation
    await this.emitSetupEvent(taskId, 'setup_started');
    
    const workspace: Workspace = {
      id: taskId,
      type: 'docker',
      environmentId: environment.id,
      status: 'initializing',
      createdAt: new Date(),
      containerId: ''
    };
    
    // TODO: Implement Docker container creation
    
    await this.emitSetupEvent(taskId, 'setup_completed');
    return workspace;
  }
  
  private async emitSetupEvent(taskId: string, subType: string): Promise<void> {
    await this.eventService.emit({
      eventId: crypto.randomUUID(),
      type: 'workspace',
      subType,
      taskId,
      timestamp: new Date(),
      isError: false,
      payload: {}
    });
  }
  
  async cleanupWorkspace(taskId: string): Promise<void> {
    const workspace = await this.getWorkspace(taskId);
    if (!workspace) return;
    
    if (workspace.type === 'local' && workspace.path) {
      // Remove git worktree
      const environment = await this.environmentService.getEnvironment(workspace.environmentId);
      if (environment?.repository.type === 'local') {
        await execAsync(
          `git worktree remove "${workspace.path}" --force`,
          { cwd: environment.repository.path }
        );
      }
    } else if (workspace.type === 'docker' && workspace.containerId) {
      // Stop and remove Docker container
      await execAsync(`docker stop ${workspace.containerId}`);
      await execAsync(`docker rm ${workspace.containerId}`);
    }
    
    this.activeWorkspaces.delete(taskId);
    await this.storage.delete(`workspaces/${taskId}`);
  }
}

export function createWorkspaceService(
  app: FastifyInstance,
  storage: StorageBackend,
  environmentService: EnvironmentService,
  eventService: EventService
): WorkspaceService {
  return new WorkspaceService(storage, environmentService, eventService);
}
```

### Phase 7: Agent Integration

**Python source files to migrate:**
- `/moatless-swarm/swarm/agents/base.py` - Base agent class
- `/moatless-swarm/swarm/agents/claude_code.py` - Claude Code agent implementation
- `/moatless-swarm/swarm/agents/options.py` - Agent options/configuration
- `/moatless-swarm/swarm/utils/subprocess.py` - Subprocess utilities for CLI integration

```typescript
// server/services/agents/base-agent.ts
// Port from: swarm/agents/base.py
import { Event } from '@shared/types/event';

export abstract class BaseAgent {
  abstract run(message: string): AsyncGenerator<Event>;
  abstract stop(): Promise<void>;
}

// server/services/agents/claude-code-agent.ts
// Port from: swarm/agents/claude_code.py
import { spawn, ChildProcess } from 'child_process';
import { createReadStream } from 'fs';
import { createInterface } from 'readline';
import { Event } from '@shared/types/event';
import { BaseAgent } from './base-agent';
import { ClaudeCodeOptions } from './agent-options';

export class ClaudeCodeAgent extends BaseAgent {
  private process?: ChildProcess;
  private stopRequested = false;
  
  constructor(
    private taskId: string,
    private options: ClaudeCodeOptions,
    private workspaceDir?: string,
    private cliPath?: string
  ) {
    super();
    this.cliPath = cliPath || this.findCli();
  }
  
  private findCli(): string {
    // Port logic from _find_cli method
    const possiblePaths = [
      process.env.HOME + '/.npm-global/bin/claude',
      '/usr/local/bin/claude',
      process.env.HOME + '/.local/bin/claude'
    ];
    
    // TODO: Check which path exists using fs.existsSync
    return 'claude'; // Default to PATH lookup
  }
  
  async *run(message: string): AsyncGenerator<Event> {
    const args = this.buildCliArgs(message);
    
    // Spawn claude process
    this.process = spawn(this.cliPath, args, {
      cwd: this.workspaceDir,
      env: this.getEnvironment(),
      stdio: ['pipe', 'pipe', 'pipe']
    });
    
    // Create readline interface for stdout
    const rl = createInterface({
      input: this.process.stdout!,
      crlfDelay: Infinity
    });
    
    // Process each line of output
    for await (const line of rl) {
      if (this.stopRequested) break;
      
      try {
        // Parse JSON lines from claude output
        const data = JSON.parse(line);
        yield this.parseEventFromJson(data);
      } catch (e) {
        // Non-JSON output, could be debug info
        console.debug('Non-JSON output:', line);
      }
    }
    
    // Wait for process to exit
    await new Promise<void>((resolve, reject) => {
      this.process!.on('close', (code) => {
        if (code === 0 || this.stopRequested) {
          resolve();
        } else {
          reject(new Error(`Process exited with code ${code}`));
        }
      });
    });
  }
  
  private buildCliArgs(message: string): string[] {
    const args: string[] = [];
    
    // Add model if specified
    if (this.options.model) {
      args.push('--model', this.options.model);
    }
    
    // Add API key if specified
    if (this.options.apiKey) {
      args.push('--api-key', this.options.apiKey);
    }
    
    // Add the message
    args.push(message);
    
    return args;
  }
  
  private getEnvironment(): NodeJS.ProcessEnv {
    return {
      ...process.env,
      CLAUDE_API_KEY: this.options.apiKey || process.env.CLAUDE_API_KEY,
      NO_COLOR: '1' // Disable color output for easier parsing
    };
  }
  
  private parseEventFromJson(data: any): Event {
    // Transform claude output to our event format
    return {
      eventId: crypto.randomUUID(),
      type: data.type || 'message',
      taskId: this.taskId,
      timestamp: new Date(),
      isError: false,
      payload: data
    };
  }
  
  async stop(): Promise<void> {
    this.stopRequested = true;
    if (this.process && !this.process.killed) {
      this.process.kill('SIGTERM');
      
      // Give it 5 seconds to gracefully shutdown
      await new Promise<void>((resolve) => {
        const timeout = setTimeout(() => {
          if (this.process && !this.process.killed) {
            this.process.kill('SIGKILL');
          }
          resolve();
        }, 5000);
        
        this.process!.on('close', () => {
          clearTimeout(timeout);
          resolve();
        });
      });
    }
  }
}

// server/services/agent.ts
import { FastifyInstance } from 'fastify';
import { BaseAgent } from './agents/base-agent';
import { ClaudeCodeAgent } from './agents/claude-code-agent';
import { TaskService } from './task';
import { WorkspaceService } from './workspace';
import { EventService } from './event';

export class AgentService {
  private activeAgents: Map<string, BaseAgent> = new Map();
  
  constructor(
    private taskService: TaskService,
    private workspaceService: WorkspaceService,
    private eventService: EventService
  ) {}
  
  async runAgent(taskId: string, message: string): Promise<void> {
    // Get task metadata
    const task = await this.taskService.getTask(taskId);
    if (!task) {
      throw new Error(`Task ${taskId} not found`);
    }
    
    // Get workspace
    const workspace = await this.workspaceService.getWorkspace(taskId);
    if (!workspace) {
      throw new Error(`Workspace for task ${taskId} not found`);
    }
    
    // Create agent based on type
    const agent = this.createAgent(task.agentType, taskId, workspace.path);
    this.activeAgents.set(taskId, agent);
    
    try {
      // Run agent and stream events
      for await (const event of agent.run(message)) {
        await this.eventService.emit(event);
      }
    } finally {
      this.activeAgents.delete(taskId);
    }
  }
  
  private createAgent(agentType: string, taskId: string, workspacePath?: string): BaseAgent {
    switch (agentType) {
      case 'claude-code':
        return new ClaudeCodeAgent(taskId, {}, workspacePath);
      default:
        throw new Error(`Unknown agent type: ${agentType}`);
    }
  }
  
  async stopAgent(taskId: string): Promise<void> {
    const agent = this.activeAgents.get(taskId);
    if (agent) {
      await agent.stop();
      this.activeAgents.delete(taskId);
    }
  }
}

export function createAgentService(
  app: FastifyInstance,
  taskService: TaskService,
  workspaceService: WorkspaceService,
  eventService: EventService
): AgentService {
  return new AgentService(taskService, workspaceService, eventService);
}
```

### Phase 8: Utility Functions

**Python source files to migrate:**
- `/moatless-swarm/swarm/utils/git_utils.py` - Git utilities
- `/moatless-swarm/swarm/utils/file_search.py` - File search utilities
- `/moatless-swarm/swarm/utils/subprocess.py` - Process management utilities

```typescript
// server/utils/git-utils.ts
// Port from: swarm/utils/git_utils.py
import { exec } from 'child_process';
import { promisify } from 'util';

const execAsync = promisify(exec);

export interface GitDiffFile {
  status: string;
  filename: string;
}

export interface GitDiffResponse {
  files: GitDiffFile[];
  totalChanges: number;
}

export async function getGitDiffFiles(repoPath: string): Promise<GitDiffResponse> {
  try {
    const { stdout } = await execAsync('git diff --name-status', {
      cwd: repoPath
    });
    
    // Parse diff output
    const files = stdout
      .split('\n')
      .filter(line => line.trim())
      .map(line => {
        const [status, filename] = line.split('\t');
        return { status, filename };
      });
      
    return { files, totalChanges: files.length };
  } catch (error) {
    throw new Error(`Git diff failed: ${error.message}`);
  }
}

export async function getCurrentBranch(repoPath: string): Promise<string> {
  const { stdout } = await execAsync('git branch --show-current', {
    cwd: repoPath
  });
  return stdout.trim();
}

export async function createBranch(repoPath: string, branchName: string): Promise<void> {
  await execAsync(`git checkout -b "${branchName}"`, {
    cwd: repoPath
  });
}

// server/utils/file-search.ts
// Port from: swarm/utils/file_search.py
import { promises as fs } from 'fs';
import path from 'path';
import { glob } from 'glob';

export interface FileSearchResponse {
  files: string[];
  count: number;
}

export async function searchFilesInDirectory(
  directory: string,
  query: string,
  limit: number = 100
): Promise<FileSearchResponse> {
  // Use glob for efficient file searching
  const pattern = `**/*${query}*`;
  const files = await glob(pattern, {
    cwd: directory,
    nodir: true,
    ignore: ['**/node_modules/**', '**/.git/**'],
    absolute: true,
    maxDepth: 10
  });
  
  // Limit results
  const limitedFiles = files.slice(0, limit);
  
  return {
    files: limitedFiles,
    count: limitedFiles.length
  };
}

export async function fileExists(filePath: string): Promise<boolean> {
  try {
    await fs.access(filePath);
    return true;
  } catch {
    return false;
  }
}

export async function readFile(filePath: string): Promise<string> {
  return fs.readFile(filePath, 'utf-8');
}

export async function writeFile(filePath: string, content: string): Promise<void> {
  const dir = path.dirname(filePath);
  await fs.mkdir(dir, { recursive: true });
  await fs.writeFile(filePath, content, 'utf-8');
}
```

### Phase 9: Service Integration

```typescript
// server/services/index.ts
import { FastifyInstance } from 'fastify';
import { createStorageService } from './storage';
import { createEventService } from './event';
import { createEnvironmentService } from './environment';
import { createWorkspaceService } from './workspace';
import { createTaskService } from './task';
import { createAgentService } from './agent';

export interface Services {
  storage: StorageBackend;
  eventService: EventService;
  environmentService: EnvironmentService;
  workspaceService: WorkspaceService;
  taskService: TaskService;
  agentService: AgentService;
}

export function setupServices(app: FastifyInstance): Services {
  // Create services with dependency injection
  const storage = createStorageService(app);
  const eventService = createEventService(app, storage);
  const environmentService = createEnvironmentService(app, storage);
  const workspaceService = createWorkspaceService(app, storage, environmentService, eventService);
  const taskService = createTaskService(app, storage, eventService, environmentService, workspaceService);
  const agentService = createAgentService(app, taskService, workspaceService, eventService);
  
  return {
    storage,
    eventService,
    environmentService,
    workspaceService,
    taskService,
    agentService
  };
}

// Extend Fastify instance type
declare module 'fastify' {
  interface FastifyInstance {
    services: Services;
  }
}
```

### Phase 10: Client Setup

```typescript
// client/vite.config.ts
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import path from 'path';
import { TanStackRouterVite } from '@tanstack/router-plugin/vite';

export default defineConfig({
  plugins: [
    TanStackRouterVite(),
    react()
  ],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
      '@shared': path.resolve(__dirname, '../shared')
    }
  },
  server: {
    port: 5173,
    proxy: {
      '/api': {
        target: 'http://localhost:3000',
        changeOrigin: true
      },
      '/ws': {
        target: 'ws://localhost:3000',
        ws: true
      }
    }
  },
  build: {
    outDir: 'dist',
    sourcemap: true
  }
});

// client/src/main.tsx
import React from 'react';
import ReactDOM from 'react-dom/client';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider, createRouter } from '@tanstack/react-router';
import { routeTree } from './routeTree.gen';
import './index.css';

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 1000 * 60 * 5, // 5 minutes
      gcTime: 1000 * 60 * 10, // 10 minutes (formerly cacheTime)
      retry: 3,
      retryDelay: attemptIndex => Math.min(1000 * 2 ** attemptIndex, 30000)
    }
  }
});

const router = createRouter({
  routeTree,
  context: {
    queryClient
  },
  defaultPreload: 'intent',
  defaultPreloadStaleTime: 0
});

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router;
  }
}

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  </React.StrictMode>
);

// client/package.json dependencies to include
{
  "dependencies": {
    "@tanstack/react-query": "^5.80.7",
    "@tanstack/react-query-devtools": "^5.80.7",
    "@tanstack/react-router": "^1.121.0",
    "@tanstack/react-router-devtools": "^1.121.0",
    "@tanstack/router-plugin": "^1.121.0",
    "react": "^19.0.0",
    "react-dom": "^19.0.0",
    "clsx": "^2.1.1",
    "tailwind-merge": "^3.3.1",
    "lucide-react": "^0.515.0",
    "zod": "^3.25.61"
  },
  "devDependencies": {
    "@tailwindcss/vite": "^4.0.6",
    "tailwindcss": "^4.0.6",
    "@vitejs/plugin-react": "^4.5.2",
    "vite": "^6.1.0",
    "vite-tsconfig-paths": "^5.1.4"
  }
}
```

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

### Quality Gates - Run After Each Task
After completing each task, verify:
```bash
# 1. Type checking passes
pnpm type:check

# 2. Linting passes
pnpm lint

# 3. Tests pass
pnpm test

# 4. Build succeeds
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

### Phase 1: Foundation & Setup (Week 1)
- [ ] Create project directory structure
- [ ] Initialize npm project and configure package.json
- [ ] Set up TypeScript configuration with strict mode
- [ ] Configure Fastify server with basic plugins
- [ ] Set up client with Vite and React
- [ ] Configure Biome for linting and formatting with naming conventions
- [ ] Set up Vitest testing framework with UI and coverage
- [ ] Create NPX entry point script
- [ ] Set up Git hooks for code quality (Husky + lint-staged)
- [ ] Configure commit linting with commitlint
- [ ] Set up Turbo for monorepo task orchestration
- [ ] Configure CI pipeline to run quality gates on every push

### Phase 2: Type System (Week 2)
- [ ] Create `shared/types` directory structure
- [ ] Port `swarm/tasks/schema.py` → `task.ts`
- [ ] Port `swarm/eventstream/events.py` → `event.ts`
- [ ] Port `swarm/environments/environments.py` → `environment.ts`
- [ ] Port `swarm/workspaces/models.py` → `workspace.ts`
- [ ] Port `swarm/workspaces/schema.py` → `workspace-schema.ts`
- [ ] Create API request/response types with Zod validation
- [ ] Create shared error types and constants
- [ ] Add shared utilities and validation functions
- [ ] Generate type documentation

### Phase 3: Server Foundation (Week 3)
- [ ] Create Fastify server setup with logging
- [ ] Configure CORS and static file serving
- [ ] Set up WebSocket support
- [ ] Create service architecture with dependency injection
- [ ] Port `swarm/storage/base.py` → `storage.ts`
- [ ] Port `swarm/storage/file.py` → file storage implementation
- [ ] Implement health check endpoint
- [ ] Create error handling middleware
- [ ] Add request validation with Zod
- [ ] Create server unit tests

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

### Phase 6: Workspace Management (Week 6)
- [ ] Port `swarm/workspaces/workspace_manager.py` → `workspace.ts`
- [ ] Port `swarm/workspaces/base.py` → base workspace logic
- [ ] Port `swarm/workspaces/local.py` → local workspace implementation
- [ ] Implement git worktree management
- [ ] Port `swarm/workspaces/docker.py` → Docker workspace support
- [ ] Add workspace cleanup utilities
- [ ] Create workspace state tracking
- [ ] Implement workspace isolation
- [ ] Add workspace unit tests
- [ ] Create workspace integration tests

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
- Rate limiting with @fastify/rate-limit
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

### 7. Development Workflow
```bash
# Development
pnpm run dev              # Runs both server and client
pnpm run dev:server       # Server only
pnpm run dev:client       # Client only

# Testing
pnpm test                 # Run all tests
pnpm test:watch           # Run tests in watch mode
pnpm test:ui              # Run tests with UI and coverage
pnpm test:coverage        # Run tests with coverage report

# Code Quality with Biome
pnpm biome:check          # Check for issues
pnpm biome:fix            # Auto-fix issues
pnpm biome:ci             # CI mode checking

# Type Checking
pnpm type:check           # Check TypeScript types

# All Checks (using Turbo)
pnpm check:turbo          # Run all checks in parallel

# Production
pnpm run build            # Build both server and client
pnpm start                # Start production server

# NPX usage
npx moatless-app         # Run the application
```

## File Migration Reference

### Priority 1 - Core Types (Week 2)
- `swarm/tasks/schema.py` → `shared/types/task.types.ts`
- `swarm/eventstream/events.py` → `shared/types/event.types.ts`
- `swarm/environments/environments.py` → `shared/types/environment.types.ts`
- `swarm/workspaces/models.py` → `shared/types/workspace.types.ts`

### Priority 2 - Server Foundation (Week 3)
- `swarm/storage/base.py` → `server/services/storage.ts`
- `swarm/storage/file.py` → `server/services/storage.ts`
- `swarm/main.py` → `server/index.ts` + `server/app.ts`

### Priority 3 - Core Services (Week 4)
- `swarm/eventstream/event_stream.py` → `server/services/event.ts`
- `swarm/eventstream/file_stream.py` → `server/services/event.ts`
- `swarm/tasks/task_manager.py` → `server/services/task.ts`
- `swarm/environments/environment_manager.py` → `server/services/environment.ts`

### Priority 4 - API Routes (Week 5)
- `swarm/api/tasks.py` → `server/routes/tasks.ts`
- `swarm/api/environments.py` → `server/routes/environments.ts`
- `swarm/api/workspaces.py` → `server/routes/workspaces.ts`

### Priority 5 - Workspaces (Week 6)
- `swarm/workspaces/workspace_manager.py` → `server/services/workspace.ts`
- `swarm/workspaces/base.py` → `server/services/workspace.ts`
- `swarm/workspaces/local.py` → `server/services/workspace.ts`
- `swarm/workspaces/docker.py` → `server/services/workspace.ts`

### Priority 6 - Agents (Week 7)
- `swarm/agents/base.py` → `server/services/agents/base-agent.ts`
- `swarm/agents/claude_code.py` → `server/services/agents/claude-code-agent.ts`
- `swarm/agents/options.py` → `server/services/agents/agent-options.ts`

### Priority 7 - Utilities (Week 8)
- `swarm/utils/git_utils.py` → `server/utils/git-utils.ts`
- `swarm/utils/file_search.py` → `server/utils/file-search.ts`
- `swarm/utils/subprocess.py` → `server/utils/process-utils.ts`

## Summary

This migration strategy transforms the Python-based Moatless Swarm into a modern TypeScript application using:
- **Fastify** for high-performance HTTP/WebSocket server
- **React** for the client-side UI
- **File-based storage** maintaining compatibility with the Python version
- **NPX distribution** for easy installation and usage
- **Monolithic architecture** simplifying deployment and distribution

The resulting application will be:
- Easy to install: `npx moatless-app`
- Cross-platform compatible (Windows, macOS, Linux)
- Performant with async/await throughout
- Type-safe with full TypeScript coverage
- Well-tested with comprehensive test suites
- Production-ready with proper error handling and logging
