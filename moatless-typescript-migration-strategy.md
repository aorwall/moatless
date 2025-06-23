# Moatless Swarm TypeScript Migration Strategy

## Executive Summary

This document outlines a comprehensive strategy for migrating the Moatless Swarm backend Python code to TypeScript for a Tauri-based desktop application. The strategy focuses on a direct port of the Python backend logic to TypeScript, eliminating the need for API calls and enabling native desktop performance.

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

## Migration Strategy

### Phase 1: Monorepo Structure Setup

Create a monorepo structure using Nx or Turborepo to manage the desktop application:

```
moatless-desktop/
├── apps/
│   └── desktop/          # Tauri desktop app
├── packages/
│   ├── core/             # Core business logic (TypeScript)
│   ├── types/            # Shared TypeScript types
│   ├── storage/          # Storage implementations
│   ├── events/           # Event system
│   ├── workspaces/       # Workspace management
│   ├── tasks/            # Task management
│   ├── environments/     # Environment management
│   └── agents/           # Agent implementations
└── tools/
    └── scripts/          # Build and migration scripts
```

### Phase 2: Type System Migration

#### 2.1 Shared Types Package

**Python source files to migrate:**
- `/moatless-swarm/swarm/tasks/schema.py` - Task-related types
- `/moatless-swarm/swarm/eventstream/events.py` - Event types
- `/moatless-swarm/swarm/environments/environments.py` - Environment types
- `/moatless-swarm/swarm/workspaces/schema.py` - Workspace types
- `/moatless-swarm/swarm/workspaces/models.py` - Workspace models

Create a `@moatless/types` package that defines all shared interfaces:

#### Naming Conventions

**File Naming:**
- Use PascalCase for files containing classes/types: `TaskManager.ts`, `EventStream.ts`
- Use camelCase for files containing functions/utilities: `gitUtils.ts`, `fileSearch.ts`
- Use dot notation for specialized files: `Task.types.ts`, `Task.test.ts`
- Index files should export public API: `index.ts`

**Directory Structure:**
```
packages/
├── types/
│   ├── src/
│   │   ├── index.ts              # Public exports
│   │   ├── Task.types.ts         # Task-related types
│   │   ├── Event.types.ts        # Event types
│   │   └── Environment.types.ts  # Environment types
│   └── package.json
├── storage/
│   ├── src/
│   │   ├── index.ts              # Public exports
│   │   ├── StorageBackend.ts     # Interface definition
│   │   ├── FileStorage.ts        # File implementation
│   │   └── TaskStorage.ts        # Task-specific storage
│   └── package.json
└── tasks/
    ├── src/
    │   ├── index.ts              # Public exports
    │   ├── TaskManager.ts        # Main class
    │   ├── TaskManager.test.ts   # Unit tests
    │   └── utils/                # Internal utilities
    │       └── responseBuilder.ts
    └── package.json
```

**Code Naming:**
- Classes: PascalCase - `TaskManager`, `LocalWorkspace`
- Interfaces/Types: PascalCase with 'I' prefix optional - `ITaskMetadata` or `TaskMetadata`
- Enums: PascalCase - `EventType`, `WorkspaceStatus`
- Functions: camelCase - `createTask`, `searchFiles`
- Constants: UPPER_SNAKE_CASE - `MAX_RETRY_COUNT`, `DEFAULT_TIMEOUT`
- Private members: underscore prefix - `_backgroundTasks`, `_stopRequested`

**Import/Export Conventions:**
```typescript
// Use named exports for better tree-shaking
export { TaskManager } from './TaskManager';
export type { ITaskMetadata } from './Task.types';

// Avoid default exports except for React components
// Bad: export default TaskManager;
// Good: export { TaskManager };

// Group imports by type
import { shell, fs } from '@tauri-apps/api';  // External
import { IStorageBackend } from '@moatless/storage';  // Internal packages
import { validateTaskId } from './utils/validation';  // Local
```

```typescript
// packages/types/src/Task.types.ts
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

// packages/types/src/Event.types.ts
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

### Phase 3: Core Components Migration

#### 3.1 Storage Layer (`@moatless/storage`)

**Python source files to migrate:**
- `/moatless-swarm/swarm/storage/base.py` - Storage interface
- `/moatless-swarm/swarm/storage/file.py` - File storage implementation
- `/moatless-swarm/swarm/tasks/task_storage.py` - Task-specific storage

```typescript
// packages/storage/src/StorageBackend.ts
// Port from: swarm/storage/base.py
export interface IStorageBackend {
  save(key: string, data: any): Promise<void>;
  get(key: string): Promise<any | null>;
  list(prefix?: string): Promise<string[]>;
  delete(key: string): Promise<void>;
  exists(key: string): Promise<boolean>;
}

// packages/storage/src/FileStorage.ts
// Port from: swarm/storage/file.py
import { fs, path } from '@tauri-apps/api';

export class FileStorage implements IStorageBackend {
  constructor(private basePath: string, private prefix: string) {}
  
  async save(key: string, data: any): Promise<void> {
    const filePath = await this.getPath(key);
    await fs.writeTextFile(filePath, JSON.stringify(data, null, 2));
  }
  
  async get(key: string): Promise<any | null> {
    try {
      const filePath = await this.getPath(key);
      const content = await fs.readTextFile(filePath);
      return JSON.parse(content);
    } catch {
      return null;
    }
  }
  
  private async getPath(key: string): Promise<string> {
    const dir = await path.join(this.basePath, this.prefix);
    await fs.createDir(dir, { recursive: true });
    return path.join(dir, `${key}.json`);
  }
}
```

#### 3.2 Event System (`@moatless/events`)

**Python source files to migrate:**
- `/moatless-swarm/swarm/eventstream/event_stream.py` - Event stream management
- `/moatless-swarm/swarm/eventstream/file_stream.py` - File-based event storage
- `/moatless-swarm/swarm/eventstream/events.py` - Event types and models

```typescript
// packages/events/src/EventStream.ts
// Port from: swarm/eventstream/event_stream.py
export class EventStream {
  private _storage: IStorageBackend;
  private _subscribers: Map<string, Set<EventHandler>> = new Map();
  
  constructor(storage: IStorageBackend) {
    this._storage = storage;
  }
  
  async emit(event: Event): Promise<void> {
    // Store event to file (similar to file_stream.py)
    const eventsKey = `events/${event.taskId}`;
    const existingEvents = await this._storage.get(eventsKey) || [];
    existingEvents.push(event);
    await this._storage.save(eventsKey, existingEvents);
    
    // Notify subscribers
    const handlers = this._subscribers.get(event.taskId) || new Set();
    for (const handler of handlers) {
      await handler(event);
    }
  }
  
  async getEvents(taskId: string, limit?: number, offset: number = 0): Promise<Event[]> {
    const events = await this._storage.get(`events/${taskId}`) || [];
    return events.slice(offset, limit ? offset + limit : undefined);
  }
  
  subscribe(taskId: string, handler: EventHandler): () => void {
    const handlers = this._subscribers.get(taskId) || new Set();
    handlers.add(handler);
    this._subscribers.set(taskId, handlers);
    
    return () => {
      handlers.delete(handler);
    };
  }
}
```

#### 3.3 Task Management (`@moatless/tasks`)

**Python source files to migrate:**
- `/moatless-swarm/swarm/tasks/task_manager.py` - Main task management logic
- `/moatless-swarm/swarm/tasks/task_storage.py` - Task storage operations
- `/moatless-swarm/swarm/tasks/response_builder.py` - Response building logic

```typescript
// packages/tasks/src/TaskManager.ts
// Port from: swarm/tasks/task_manager.py
export class TaskManager {
  private _backgroundTasks = new Set<Promise<void>>();
  
  constructor(
    private taskStorage: TaskStorage,
    private environmentStorage: EnvironmentManager,
    private eventStreamStorage: EventStream,
    private workspaceManager: WorkspaceManager
  ) {}
  
  async createTask(params: CreateTaskParams): Promise<string> {
    // Validate environment exists
    await this.validateEnvironmentExists(params.environmentId);
    
    const taskId = crypto.randomUUID();
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
    
    await this.taskStorage.saveMetadata(taskId, metadata);
    
    // Setup workspace asynchronously in background
    const setupTask = this.setupWorkspaceAsync(
      taskId, 
      metadata, 
      params.branchName, 
      params.message, 
      params.start
    );
    
    this._backgroundTasks.add(setupTask);
    setupTask.finally(() => this._backgroundTasks.delete(setupTask));
    
    return taskId;
  }
  
  async sendMessage(taskId: string, message: string): Promise<string> {
    // Port logic from task_manager.py send_message method
    const metadata = await this.taskStorage.getMetadata(taskId);
    if (!metadata) {
      throw new Error(`Task ${taskId} not found`);
    }
    
    // Continue implementation...
  }
}
```

### Phase 4: Environment Management

**Python source files to migrate:**
- `/moatless-swarm/swarm/environments/environment_manager.py` - Environment management
- `/moatless-swarm/swarm/environments/environments.py` - Environment models

```typescript
// packages/environments/src/EnvironmentManager.ts
// Port from: swarm/environments/environment_manager.py
import { IStorageBackend } from '@moatless/storage';
import { Environment, LocalRepository, FileSearchRequest, FileSearchResponse } from '@moatless/types';

export class EnvironmentManager {
  constructor(private _storage: IStorageBackend) {}
  
  async saveEnvironment(env: Environment): Promise<void> {
    await this._storage.save(env.id, env);
  }
  
  async getEnvironment(envId: string): Promise<Environment | null> {
    return await this._storage.get(envId);
  }
  
  async listEnvironments(): Promise<Environment[]> {
    const keys = await this._storage.list();
    const environments: Environment[] = [];
    
    for (const key of keys) {
      const env = await this._storage.get(key);
      if (env) environments.push(env);
    }
    
    return environments;
  }
}
```

### Phase 5: Workspace Management

**Python source files to migrate:**
- `/moatless-swarm/swarm/workspaces/workspace_manager.py` - Workspace management
- `/moatless-swarm/swarm/workspaces/base.py` - Base workspace class
- `/moatless-swarm/swarm/workspaces/local.py` - Local workspace implementation
- `/moatless-swarm/swarm/workspaces/docker.py` - Docker workspace implementation
- `/moatless-swarm/swarm/workspaces/server.py` - Server management

```typescript
// packages/workspaces/src/WorkspaceManager.ts
// Port from: swarm/workspaces/workspace_manager.py
export class WorkspaceManager {
  private _activeWorkspaces: Map<string, BaseWorkspace> = new Map();
  
  constructor(
    private environmentStorage: EnvironmentManager,
    private eventStreamStorage: EventStream,
    private taskStorage: TaskStorage
  ) {}
  
  async getOrCreateWorkspace(
    taskId: string, 
    metadata: TaskMetadata, 
    branchName?: string
  ): Promise<BaseWorkspace> {
    // Check if workspace already exists
    const existing = this._activeWorkspaces.get(taskId);
    if (existing) return existing;
    
    // Get environment
    const environment = await this.environmentStorage.getEnvironment(metadata.environmentId);
    if (!environment) {
      throw new Error(`Environment not found: ${metadata.environmentId}`);
    }
    
    // Create workspace based on code execution type
    let workspace: BaseWorkspace;
    if (environment.codeExecution.type === 'docker') {
      workspace = new DockerWorkspace(taskId, environment, this.eventStreamStorage);
    } else {
      const originalBranch = environment.repository.type === 'local' 
        ? environment.repository.originalBranch 
        : undefined;
      workspace = new LocalWorkspace(
        taskId, 
        environment, 
        this.eventStreamStorage, 
        originalBranch, 
        branchName
      );
    }
    
    this._activeWorkspaces.set(taskId, workspace);
    return workspace;
  }
}

// packages/workspaces/src/LocalWorkspace.ts
// Port from: swarm/workspaces/local.py
import { shell } from '@tauri-apps/api';

export class LocalWorkspace extends BaseWorkspace {
  async setup(): Promise<void> {
    await this.emitSetupEvent('setup_started');
    
    if (this.environment.repository.type === 'local') {
      // Create git worktree using Tauri shell
      const worktreePath = await this.getWorktreePath();
      const result = await shell.execute('git', [
        'worktree', 
        'add', 
        worktreePath,
        this.branchName || 'HEAD'
      ]);
      
      if (result.code !== 0) {
        throw new Error(`Failed to create worktree: ${result.stderr}`);
      }
    }
    
    await this.emitSetupEvent('setup_completed');
  }
}
```

### Phase 6: Agent Integration

**Python source files to migrate:**
- `/moatless-swarm/swarm/agents/base.py` - Base agent class
- `/moatless-swarm/swarm/agents/claude_code.py` - Claude Code agent implementation
- `/moatless-swarm/swarm/agents/options.py` - Agent options/configuration
- `/moatless-swarm/swarm/utils/subprocess.py` - Subprocess utilities for CLI integration

```typescript
// packages/agents/src/BaseAgent.ts
// Port from: swarm/agents/base.py
export abstract class BaseAgent {
  abstract run(message: string): AsyncGenerator<Event>;
  abstract stop(): Promise<void>;
}

// packages/agents/src/ClaudeCodeAgent.ts
// Port from: swarm/agents/claude_code.py
import { shell, Child } from '@tauri-apps/api/shell';
import { ClaudeCodeOptions } from './options';

export class ClaudeCodeAgent extends BaseAgent {
  private _process?: Child;
  private _stopRequested = false;
  
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
      '~/.npm-global/bin/claude',
      '/usr/local/bin/claude',
      '~/.local/bin/claude'
    ];
    // Check which path exists using Tauri fs API
    return 'claude'; // Default to PATH lookup
  }
  
  async *run(message: string): AsyncGenerator<Event> {
    const args = this.buildCliArgs(message);
    
    // Create command using Tauri shell
    const command = new shell.Command('claude', args, {
      cwd: this.workspaceDir,
      env: this.getEnvironment()
    });
    
    // Set up event listeners for stdout
    command.on('close', (data) => {
      console.log(`Command finished with code ${data.code}`);
    });
    
    command.stdout.on('data', (line: string) => {
      // Parse JSON lines and yield events
      try {
        const data = JSON.parse(line);
        yield this.parseEventFromJson(data);
      } catch (e) {
        console.error('Failed to parse JSON:', line);
      }
    });
    
    // Execute command
    this._process = await command.spawn();
  }
  
  async stop(): Promise<void> {
    this._stopRequested = true;
    if (this._process) {
      await this._process.kill();
    }
  }
}
```

### Phase 7: Utility Functions

**Python source files to migrate:**
- `/moatless-swarm/swarm/utils/git_utils.py` - Git utilities
- `/moatless-swarm/swarm/utils/file_search.py` - File search utilities
- `/moatless-swarm/swarm/utils/subprocess.py` - Process management utilities

```typescript
// packages/utils/src/gitUtils.ts
// Port from: swarm/utils/git_utils.py
import { shell } from '@tauri-apps/api';

export async function getGitDiffFiles(repoPath: string): Promise<GitDiffResponse> {
  const result = await shell.execute('git', ['diff', '--name-status'], {
    cwd: repoPath
  });
  
  if (result.code !== 0) {
    throw new Error(`Git diff failed: ${result.stderr}`);
  }
  
  // Parse diff output
  const files = result.stdout.split('\n')
    .filter(line => line.trim())
    .map(line => {
      const [status, filename] = line.split('\t');
      return { status, filename };
    });
    
  return { files, totalChanges: files.length };
}

// packages/utils/src/fileSearch.ts
// Port from: swarm/utils/file_search.py
import { fs, path } from '@tauri-apps/api';

export async function searchFilesInDirectory(
  directory: string,
  query: string,
  limit: number = 100
): Promise<FileSearchResponse> {
  const results: string[] = [];
  
  async function walkDir(dir: string) {
    const entries = await fs.readDir(dir);
    
    for (const entry of entries) {
      if (results.length >= limit) break;
      
      const fullPath = await path.join(dir, entry.name);
      
      if (entry.children) {
        // It's a directory, recurse
        await walkDir(fullPath);
      } else if (entry.name.toLowerCase().includes(query.toLowerCase())) {
        results.push(fullPath);
      }
    }
  }
  
  await walkDir(directory);
  return { files: results, count: results.length };
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

### Phase 1: Foundation & Setup (Week 1)
- [ ] Set up monorepo structure with Nx/Turborepo
- [ ] Create Tauri desktop app scaffold with basic window
- [ ] Configure TypeScript with strict mode and path aliases
- [ ] Set up Vite build pipeline for Tauri
- [ ] Configure ESLint and Prettier with naming conventions
- [ ] Set up Jest/Vitest testing framework
- [ ] Create base package.json structure for all packages
- [ ] Set up Git hooks for code quality (Husky + lint-staged)

### Phase 2: Type System (Week 2)
- [ ] Create `@moatless/types` package
- [ ] Port `swarm/tasks/schema.py` → `Task.types.ts`
- [ ] Port `swarm/eventstream/events.py` → `Event.types.ts`
- [ ] Port `swarm/environments/environments.py` → `Environment.types.ts`
- [ ] Port `swarm/workspaces/models.py` → `Workspace.types.ts`
- [ ] Port `swarm/workspaces/schema.py` → `WorkspaceSchema.types.ts`
- [ ] Create shared error types
- [ ] Add Zod schemas for runtime validation
- [ ] Generate type documentation

### Phase 3: Storage Layer (Week 2)
- [ ] Create `@moatless/storage` package
- [ ] Port `swarm/storage/base.py` → `StorageBackend.ts`
- [ ] Port `swarm/storage/file.py` → `FileStorage.ts`
- [ ] Port `swarm/tasks/task_storage.py` → `TaskStorage.ts`
- [ ] Implement Tauri FS API integration
- [ ] Add storage migration utilities
- [ ] Create storage unit tests
- [ ] Add storage performance benchmarks

### Phase 4: Event System (Weeks 3-4)
- [ ] Create `@moatless/events` package
- [ ] Port `swarm/eventstream/event_stream.py` → `EventStream.ts`
- [ ] Port `swarm/eventstream/file_stream.py` → `FileEventStream.ts`
- [ ] Implement event subscription system
- [ ] Add event replay functionality
- [ ] Create event filtering utilities
- [ ] Implement event compression for storage
- [ ] Add event stream unit tests
- [ ] Create event debugging tools

### Phase 5: Environment Management (Week 4)
- [ ] Create `@moatless/environments` package
- [ ] Port `swarm/environments/environment_manager.py` → `EnvironmentManager.ts`
- [ ] Implement environment validation
- [ ] Add environment templates
- [ ] Create environment import/export
- [ ] Add environment unit tests

### Phase 6: Task Management (Weeks 5-6)
- [ ] Create `@moatless/tasks` package
- [ ] Port `swarm/tasks/task_manager.py` → `TaskManager.ts`
- [ ] Port `swarm/tasks/response_builder.py` → `TaskResponseBuilder.ts`
- [ ] Implement task lifecycle state machine
- [ ] Add task queuing system
- [ ] Create task event handlers
- [ ] Implement task persistence
- [ ] Add background task management
- [ ] Create task unit tests
- [ ] Add task integration tests

### Phase 7: Workspace Management (Weeks 7-8)
- [ ] Create `@moatless/workspaces` package
- [ ] Port `swarm/workspaces/workspace_manager.py` → `WorkspaceManager.ts`
- [ ] Port `swarm/workspaces/base.py` → `BaseWorkspace.ts`
- [ ] Port `swarm/workspaces/local.py` → `LocalWorkspace.ts`
- [ ] Port `swarm/workspaces/docker.py` → `DockerWorkspace.ts`
- [ ] Port `swarm/workspaces/server.py` → `ServerManager.ts`
- [ ] Implement git worktree management
- [ ] Add workspace cleanup utilities
- [ ] Create workspace state tracking
- [ ] Implement server process management
- [ ] Add workspace unit tests
- [ ] Create workspace integration tests

### Phase 8: Agent Integration (Weeks 9-10)
- [ ] Create `@moatless/agents` package
- [ ] Port `swarm/agents/base.py` → `BaseAgent.ts`
- [ ] Port `swarm/agents/claude_code.py` → `ClaudeCodeAgent.ts`
- [ ] Port `swarm/agents/options.py` → `ClaudeCodeOptions.ts`
- [ ] Implement CLI process spawning with Tauri
- [ ] Add stdout/stderr stream parsing
- [ ] Create agent state management
- [ ] Implement agent timeout handling
- [ ] Add agent retry logic
- [ ] Create agent unit tests
- [ ] Add Claude CLI integration tests

### Phase 9: Utilities (Week 11)
- [ ] Create `@moatless/utils` package
- [ ] Port `swarm/utils/git_utils.py` → `gitUtils.ts`
- [ ] Port `swarm/utils/file_search.py` → `fileSearch.ts`
- [ ] Port `swarm/utils/subprocess.py` → `subprocess.ts`
- [ ] Add path manipulation utilities
- [ ] Create async utility functions
- [ ] Implement rate limiting utilities
- [ ] Add logging utilities
- [ ] Create utility unit tests

### Phase 10: Integration & UI (Week 11)
- [ ] Create main Tauri application entry point
- [ ] Integrate all packages into desktop app
- [ ] Create IPC command handlers
- [ ] Implement frontend-backend communication
- [ ] Add progress tracking UI
- [ ] Create error handling UI
- [ ] Implement settings management
- [ ] Add keyboard shortcuts
- [ ] Create application menu

### Phase 11: Testing & Quality (Week 12)
- [ ] Set up end-to-end testing with Playwright
- [ ] Create integration test suite
- [ ] Add performance benchmarks
- [ ] Implement memory leak detection
- [ ] Create stress tests
- [ ] Add security audit
- [ ] Set up CI/CD pipeline
- [ ] Create automated release process

### Phase 12: Documentation & Deployment (Week 12)
- [ ] Create user documentation
- [ ] Write API documentation
- [ ] Create migration guide from Python version
- [ ] Add troubleshooting guide
- [ ] Create developer onboarding docs
- [ ] Set up auto-updating mechanism
- [ ] Create installer for Windows/Mac/Linux
- [ ] Add telemetry (with user consent)
- [ ] Create feedback mechanism

### Additional Tasks (As Discovered)
- [ ] _Space for new tasks discovered during migration_
- [ ] _Add items here as needed_

## Technical Considerations

### 1. State Management
- Use Zustand for application state
- Implement file-based persistence for task data
- Event sourcing for task history

### 2. Error Handling
```typescript
export class MoatlessError extends Error {
  constructor(
    message: string,
    public code: string,
    public details?: any
  ) {
    super(message);
  }
}

export class TaskNotFoundError extends MoatlessError {
  constructor(taskId: string) {
    super(`Task ${taskId} not found`, 'TASK_NOT_FOUND');
  }
}

export class WorkspaceError extends MoatlessError {
  constructor(message: string, taskId: string) {
    super(message, 'WORKSPACE_ERROR', { taskId });
  }
}
```

### 3. Type Safety
- Use Zod for runtime validation of file data
- Strict TypeScript configuration
- Explicit null checks and type guards

### 4. Performance Optimization
- Efficient file I/O with batching
- Stream processing for large event lists
- Background task management
- Memory-efficient event storage

### 5. Tauri-Specific Considerations
- Use Tauri's permission system for file access
- Implement proper IPC communication
- Handle cross-platform path differences
- Secure credential storage using Tauri's APIs


## File Migration Reference

### Priority 1 - Core Types (Week 2)
- `swarm/tasks/schema.py` → `packages/types/src/Task.types.ts`
- `swarm/eventstream/events.py` → `packages/types/src/Event.types.ts`
- `swarm/environments/environments.py` → `packages/types/src/Environment.types.ts`
- `swarm/workspaces/models.py` → `packages/types/src/Workspace.types.ts`

### Priority 2 - Storage (Week 2)
- `swarm/storage/base.py` → `packages/storage/src/StorageBackend.ts`
- `swarm/storage/file.py` → `packages/storage/src/FileStorage.ts`
- `swarm/tasks/task_storage.py` → `packages/storage/src/TaskStorage.ts`

### Priority 3 - Events (Weeks 3-4)
- `swarm/eventstream/event_stream.py` → `packages/events/src/EventStream.ts`
- `swarm/eventstream/file_stream.py` → `packages/events/src/FileEventStream.ts`

### Priority 4 - Task Management (Weeks 5-6)
- `swarm/tasks/task_manager.py` → `packages/tasks/src/TaskManager.ts`
- `swarm/tasks/response_builder.py` → `packages/tasks/src/TaskResponseBuilder.ts`
- `swarm/environments/environment_manager.py` → `packages/environments/src/EnvironmentManager.ts`

### Priority 5 - Workspaces (Weeks 7-8)
- `swarm/workspaces/workspace_manager.py` → `packages/workspaces/src/WorkspaceManager.ts`
- `swarm/workspaces/base.py` → `packages/workspaces/src/BaseWorkspace.ts`
- `swarm/workspaces/local.py` → `packages/workspaces/src/LocalWorkspace.ts`
- `swarm/workspaces/docker.py` → `packages/workspaces/src/DockerWorkspace.ts`
- `swarm/workspaces/server.py` → `packages/workspaces/src/ServerManager.ts`

### Priority 6 - Agents (Weeks 9-10)
- `swarm/agents/base.py` → `packages/agents/src/BaseAgent.ts`
- `swarm/agents/claude_code.py` → `packages/agents/src/ClaudeCodeAgent.ts`
- `swarm/agents/options.py` → `packages/agents/src/ClaudeCodeOptions.ts`

### Priority 7 - Utilities (Week 11)
- `swarm/utils/git_utils.py` → `packages/utils/src/gitUtils.ts`
- `swarm/utils/file_search.py` → `packages/utils/src/fileSearch.ts`
- `swarm/utils/subprocess.py` → `packages/utils/src/subprocess.ts`
