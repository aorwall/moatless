# Type System Migration Guide

This guide covers the migration of the Python type system to TypeScript, establishing the shared types that will be used throughout the application.

## Overview

The type system forms the foundation of the TypeScript application, providing type safety and clear contracts between the server and client components.

## Python to TypeScript Type Mapping

| Python Type | TypeScript Equivalent | Notes |
|-------------|---------------------|-------|
| `str` | `string` | |
| `int`, `float` | `number` | TypeScript doesn't distinguish between integers and floats |
| `bool` | `boolean` | |
| `Dict[str, Any]` | `Record<string, any>` | Consider more specific types when possible |
| `List[T]` | `Array<T>` or `T[]` | Both syntaxes are valid |
| `Optional[T]` | `T \| null \| undefined` | Use `T \| undefined` for optional parameters |
| `Union[A, B]` | `A \| B` | TypeScript union types |
| `datetime` | `Date` | Use ISO strings for serialization |
| `UUID` | `string` | Add validation with Zod |
| `Enum` | `enum` or union types | Prefer union types for tree-shaking |
| Pydantic Models | TypeScript interfaces | Use Zod for runtime validation |

## Directory Structure

```
shared/
├── types/
│   ├── index.ts           # Public exports
│   ├── task.ts            # Task-related types
│   ├── event.ts           # Event types
│   ├── environment.ts     # Environment types
│   ├── workspace.ts       # Workspace types
│   └── api.ts             # API request/response types
├── constants/
│   ├── index.ts
│   └── errors.ts
└── utils/
    ├── index.ts
    └── validation.ts
```

## Naming Conventions

### File Naming
- Use kebab-case for all files: `task-manager.ts`, `event-stream.ts`
- No redundant suffixes when context is clear (e.g., `types/task.ts` not `types/task.types.ts`)
- Index files should export public API: `index.ts`

### Code Naming
- **Classes**: PascalCase - `TaskManager`, `LocalWorkspace`
- **Interfaces/Types**: PascalCase without 'I' prefix - `TaskMetadata`, `Environment`
- **Enums**: PascalCase - `EventType`, `WorkspaceStatus`
- **Functions**: camelCase - `createTask`, `searchFiles`
- **Constants**: UPPER_SNAKE_CASE - `MAX_RETRY_COUNT`, `DEFAULT_TIMEOUT`
- **Private members**: prefix with # for ES2022 private fields or underscore
- **Boolean variables**: use prefixes - `isActive`, `hasError`, `shouldRetry`

## Implementation

### 1. Task Types

**Source**: `/moatless-swarm/swarm/tasks/schema.py`  
**Target**: `shared/types/task.ts`

```typescript
// shared/types/task.ts
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

export interface TaskMessage {
  taskId: string;
  message: string;
  timestamp: Date;
}

export interface TaskSummary {
  taskId: string;
  title: string;
  status: TaskMetadata['status'];
  createdAt: Date;
  agentType: string;
}
```

### 2. Event Types

**Source**: `/moatless-swarm/swarm/eventstream/events.py`  
**Target**: `shared/types/event.ts`

```typescript
// shared/types/event.ts
export enum EventType {
  INPUT = 'input',
  MESSAGE = 'message',
  START = 'start',
  STOP = 'stop',
  RESULT = 'result',
  ERROR = 'error',
  WORKSPACE = 'workspace',
  SERVER = 'server',
  TOOL = 'tool',
  COMPLETION = 'completion',
}

export interface BaseEvent {
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
}

export interface EventPayload {
  [key: string]: any;
}

export interface Event extends BaseEvent {
  payload: EventPayload;
}

// Specific event types
export interface InputEvent extends BaseEvent {
  type: EventType.INPUT;
  payload: {
    message: string;
  };
}

export interface MessageEvent extends BaseEvent {
  type: EventType.MESSAGE;
  payload: {
    content: string;
    role: 'user' | 'assistant' | 'system';
  };
}

export interface ErrorEvent extends BaseEvent {
  type: EventType.ERROR;
  isError: true;
  payload: {
    error: string;
    stackTrace?: string;
    code?: string;
  };
}

export interface WorkspaceEvent extends BaseEvent {
  type: EventType.WORKSPACE;
  subType: 'setup_started' | 'setup_completed' | 'setup_failed';
  payload: {
    workspaceId?: string;
    error?: string;
  };
}

export type SpecificEvent = InputEvent | MessageEvent | ErrorEvent | WorkspaceEvent;
```

### 3. Environment Types

**Source**: `/moatless-swarm/swarm/environments/environments.py`  
**Target**: `shared/types/environment.ts`

```typescript
// shared/types/environment.ts
export interface Environment {
  id: string;
  name: string;
  description?: string;
  repository: Repository;
  codeExecution: CodeExecution;
  createdAt: Date;
  updatedAt: Date;
}

export interface Repository {
  type: 'local' | 'github' | 'gitlab';
  path?: string;  // For local repos
  url?: string;   // For remote repos
  branch?: string;
  commit?: string;
}

export interface LocalRepository extends Repository {
  type: 'local';
  path: string;
}

export interface RemoteRepository extends Repository {
  type: 'github' | 'gitlab';
  url: string;
  accessToken?: string;
}

export interface CodeExecution {
  type: 'local' | 'docker' | 'sandbox';
  config?: Record<string, any>;
}

export interface FileSearchRequest {
  environmentId: string;
  query: string;
  filePattern?: string;
  maxResults?: number;
}

export interface FileSearchResponse {
  files: string[];
  totalCount: number;
  truncated: boolean;
}
```

### 4. Workspace Types

**Source**: `/moatless-swarm/swarm/workspaces/models.py`  
**Target**: `shared/types/workspace.ts`

```typescript
// shared/types/workspace.ts
export interface BaseWorkspace {
  id: string;
  type: 'local' | 'docker';
  environmentId: string;
  status: 'initializing' | 'ready' | 'error' | 'cleaning';
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
  image?: string;
}

export type Workspace = LocalWorkspace | DockerWorkspace;

// Type guards
export function isLocalWorkspace(workspace: Workspace): workspace is LocalWorkspace {
  return workspace.type === 'local';
}

export function isDockerWorkspace(workspace: Workspace): workspace is DockerWorkspace {
  return workspace.type === 'docker';
}

export interface WorkspaceFile {
  path: string;
  content: string;
  language?: string;
}

export interface WorkspaceChange {
  type: 'create' | 'modify' | 'delete';
  path: string;
  oldContent?: string;
  newContent?: string;
}
```

### 5. API Types with Zod Validation

**Target**: `shared/types/api.ts`

```typescript
// shared/types/api.ts
import { z } from 'zod';

// Task API schemas
export const CreateTaskRequestSchema = z.object({
  agentType: z.string(),
  environmentId: z.string(),
  message: z.string(),
  branchName: z.string().optional(),
  start: z.boolean().default(true),
});

export type CreateTaskRequest = z.infer<typeof CreateTaskRequestSchema>;

export const TaskResponseSchema = z.object({
  taskId: z.string(),
  status: z.enum(['active', 'completed', 'error', 'closed']),
  message: z.string().optional(),
});

export type TaskResponse = z.infer<typeof TaskResponseSchema>;

// Environment API schemas
export const CreateEnvironmentRequestSchema = z.object({
  name: z.string(),
  description: z.string().optional(),
  repository: z.object({
    type: z.enum(['local', 'github', 'gitlab']),
    path: z.string().optional(),
    url: z.string().optional(),
    branch: z.string().optional(),
    accessToken: z.string().optional(),
  }),
  codeExecution: z.object({
    type: z.enum(['local', 'docker', 'sandbox']),
    config: z.record(z.any()).optional(),
  }),
});

export type CreateEnvironmentRequest = z.infer<typeof CreateEnvironmentRequestSchema>;

// Message API schemas
export const SendMessageRequestSchema = z.object({
  message: z.string().min(1),
});

export type SendMessageRequest = z.infer<typeof SendMessageRequestSchema>;

// Common API response types
export interface ApiError {
  code: string;
  message: string;
  details?: any;
}

export interface PaginatedResponse<T> {
  items: T[];
  total: number;
  page: number;
  pageSize: number;
  hasMore: boolean;
}

// Query parameter schemas
export const PaginationQuerySchema = z.object({
  page: z.coerce.number().min(1).default(1),
  pageSize: z.coerce.number().min(1).max(100).default(20),
});

export type PaginationQuery = z.infer<typeof PaginationQuerySchema>;
```

### 6. Shared Constants and Errors

**Target**: `shared/constants/errors.ts`

```typescript
// shared/constants/errors.ts
export const ERROR_CODES = {
  // Task errors
  TASK_NOT_FOUND: 'TASK_NOT_FOUND',
  TASK_ALREADY_EXISTS: 'TASK_ALREADY_EXISTS',
  TASK_INVALID_STATE: 'TASK_INVALID_STATE',
  
  // Environment errors
  ENVIRONMENT_NOT_FOUND: 'ENVIRONMENT_NOT_FOUND',
  ENVIRONMENT_INVALID_CONFIG: 'ENVIRONMENT_INVALID_CONFIG',
  
  // Workspace errors
  WORKSPACE_ERROR: 'WORKSPACE_ERROR',
  WORKSPACE_SETUP_FAILED: 'WORKSPACE_SETUP_FAILED',
  
  // Agent errors
  AGENT_NOT_FOUND: 'AGENT_NOT_FOUND',
  AGENT_EXECUTION_ERROR: 'AGENT_EXECUTION_ERROR',
  
  // Validation errors
  VALIDATION_ERROR: 'VALIDATION_ERROR',
  
  // Generic errors
  INTERNAL_SERVER_ERROR: 'INTERNAL_SERVER_ERROR',
  NOT_IMPLEMENTED: 'NOT_IMPLEMENTED',
} as const;

export type ErrorCode = typeof ERROR_CODES[keyof typeof ERROR_CODES];

export class MoatlessError extends Error {
  constructor(
    message: string,
    public code: ErrorCode,
    public statusCode: number = 500,
    public details?: any
  ) {
    super(message);
    this.name = 'MoatlessError';
  }
}

export class TaskNotFoundError extends MoatlessError {
  constructor(taskId: string) {
    super(
      `Task ${taskId} not found`,
      ERROR_CODES.TASK_NOT_FOUND,
      404
    );
  }
}

export class EnvironmentNotFoundError extends MoatlessError {
  constructor(environmentId: string) {
    super(
      `Environment ${environmentId} not found`,
      ERROR_CODES.ENVIRONMENT_NOT_FOUND,
      404
    );
  }
}

export class WorkspaceError extends MoatlessError {
  constructor(message: string, taskId: string) {
    super(
      message,
      ERROR_CODES.WORKSPACE_ERROR,
      500,
      { taskId }
    );
  }
}

export class ValidationError extends MoatlessError {
  constructor(message: string, errors: any) {
    super(
      message,
      ERROR_CODES.VALIDATION_ERROR,
      400,
      { errors }
    );
  }
}
```

### 7. Utility Types and Functions

**Target**: `shared/utils/validation.ts`

```typescript
// shared/utils/validation.ts
import { z } from 'zod';

// UUID validation
export const uuidSchema = z.string().uuid();

// Date validation and parsing
export const dateSchema = z.union([
  z.string().datetime(),
  z.date(),
]).transform((val) => val instanceof Date ? val : new Date(val));

// Common validation functions
export function isValidUuid(value: string): boolean {
  return uuidSchema.safeParse(value).success;
}

export function parseDate(value: string | Date): Date {
  return dateSchema.parse(value);
}

// Type utilities
export type DeepPartial<T> = {
  [P in keyof T]?: T[P] extends object ? DeepPartial<T[P]> : T[P];
};

export type RequireAtLeastOne<T> = {
  [K in keyof T]-?: Required<Pick<T, K>> & Partial<Pick<T, Exclude<keyof T, K>>>;
}[keyof T];

// JSON serialization helpers
export function toJSON<T>(data: T): string {
  return JSON.stringify(data, (key, value) => {
    if (value instanceof Date) {
      return value.toISOString();
    }
    return value;
  });
}

export function fromJSON<T>(json: string): T {
  return JSON.parse(json, (key, value) => {
    if (typeof value === 'string' && /\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}/.test(value)) {
      return new Date(value);
    }
    return value;
  });
}
```

### 8. Index Files for Public Exports

**Target**: `shared/types/index.ts`

```typescript
// shared/types/index.ts
export * from './task';
export * from './event';
export * from './environment';
export * from './workspace';
export * from './api';
```

**Target**: `shared/constants/index.ts`

```typescript
// shared/constants/index.ts
export * from './errors';

// Additional constants
export const DEFAULT_PAGE_SIZE = 20;
export const MAX_PAGE_SIZE = 100;
export const DEFAULT_TIMEOUT = 30000; // 30 seconds
export const MAX_RETRY_COUNT = 3;
export const EVENT_BATCH_SIZE = 100;
```

**Target**: `shared/utils/index.ts`

```typescript
// shared/utils/index.ts
export * from './validation';
```

## Testing the Type System

Create tests to ensure type safety and validation:

```typescript
// shared/types/__tests__/validation.test.ts
import { describe, it, expect } from 'vitest';
import { CreateTaskRequestSchema } from '../api';

describe('API Schema Validation', () => {
  describe('CreateTaskRequestSchema', () => {
    it('should validate valid request', () => {
      const validRequest = {
        agentType: 'claude-code',
        environmentId: 'env-123',
        message: 'Hello world',
      };
      
      const result = CreateTaskRequestSchema.safeParse(validRequest);
      expect(result.success).toBe(true);
      if (result.success) {
        expect(result.data.start).toBe(true); // Default value
      }
    });
    
    it('should reject invalid request', () => {
      const invalidRequest = {
        agentType: 'claude-code',
        // Missing environmentId
        message: 'Hello world',
      };
      
      const result = CreateTaskRequestSchema.safeParse(invalidRequest);
      expect(result.success).toBe(false);
    });
  });
});
```

## Migration Checklist

- [ ] Create `shared/types` directory structure
- [ ] Port task types from `swarm/tasks/schema.py`
- [ ] Port event types from `swarm/eventstream/events.py`
- [ ] Port environment types from `swarm/environments/environments.py`
- [ ] Port workspace types from `swarm/workspaces/models.py`
- [ ] Create API request/response types with Zod validation
- [ ] Create shared error types and constants
- [ ] Add validation utilities
- [ ] Create index files for clean exports
- [ ] Write unit tests for validation schemas
- [ ] Update client code to use new types
- [ ] Document any type migration decisions

## Best Practices

1. **Prefer interfaces over types** for object shapes (better error messages)
2. **Use union types over enums** when possible (better tree-shaking)
3. **Export type guards** for discriminated unions
4. **Use Zod for runtime validation** at API boundaries
5. **Keep types close to their usage** when they're not shared
6. **Document complex types** with JSDoc comments
7. **Avoid `any`** - use `unknown` when type is truly unknown
8. **Use strict null checks** - be explicit about optional values

## Next Steps

1. Review [Server Foundation](./03-server-foundation.md) for backend setup
2. Update client imports to use the new shared types
3. Implement runtime validation at API boundaries