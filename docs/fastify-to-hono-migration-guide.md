# Fastify to Hono Migration Guide

## Overview

This guide provides a comprehensive plan for migrating the Moatless backend from Fastify to Hono framework, with a focus on enhancing SSE (Server-Sent Events) streaming capabilities using Hono's built-in `streamSSE()` helper.

## Table of Contents

1. [Migration Benefits](#migration-benefits)
2. [Test Analysis & Migration Strategy](#test-analysis--migration-strategy)
3. [Phase 1: Setup and Core Infrastructure](#phase-1-setup-and-core-infrastructure)
4. [Phase 2: Route Migration](#phase-2-route-migration)
5. [Phase 3: SSE Implementation](#phase-3-sse-implementation)
6. [Phase 4: Middleware Migration](#phase-4-middleware-migration)
7. [Phase 5: Testing Strategy](#phase-5-testing-strategy)
8. [Migration Timeline](#migration-timeline)
9. [Testing Refactoring](#testing-refactoring)

## Migration Benefits

1. **Better SSE Support**: Native `streamSSE()` helper with built-in event formatting
2. **Performance**: Lighter framework with better throughput
3. **Modern APIs**: Built on Web Standards
4. **Flexibility**: Multi-runtime support (Node.js, Bun, Deno, Edge)
5. **Simpler Streaming**: More intuitive streaming APIs

## Test Analysis & Migration Strategy

### Current Test Suite Overview

**13 test files** analyzed across backend routes with varying migration complexity:

### 🔴 **HIGH PRIORITY - Core API Tests (4 files)**
- **`error-handling.test.ts`** - **CRITICAL**: Error middleware patterns need complete rework
- **`tasks.test.ts`** - **CRITICAL**: 420 lines of core business logic tests  
- **`repositories.test.ts`** + **`repositories-e2e.test.ts`** - **CRITICAL**: Repository management
- **`workspace-endpoints.test.ts`** - **CRITICAL**: File operations and workspace management

### 🟡 **MEDIUM PRIORITY - Integration Tests (3 files)**
- **`agent-lifecycle.test.ts`** - Agent start/stop lifecycle management
- **`server-lifecycle-e2e.test.ts`** - Real server process management
- **`complete-workflow-e2e.test.ts`** - Full end-to-end workflow with Claude API

### 🟠 **SSE/STREAMING FOCUS (4 files)**
- **`events-sse-real-server.test.ts`** - **HIGH IMPACT**: Real HTTP SSE connections
- **`events-global-stream.test.ts`** - Global event streaming with SSE manager mocking
- **`events-backend-flow.test.ts`** - Event service integration patterns
- **`events-subscription.test.ts`** - Event subscription mechanisms

### 🟢 **LOW PRIORITY - Cleanup (1 file)**
- **`sse.test.ts`** - **DEPRECATED**: Tests removed endpoints, can be simplified/removed

### Key Migration Challenges Identified

#### 1. **Universal Pattern Changes**
- **ALL 12 active test files** use `app.inject()` → Need Hono `app.fetch()` equivalent
- **ALL files** use `app.services.*` pattern → Need Hono context injection
- **Service mocking strategy** needs complete rework for Hono context

#### 2. **SSE Testing Complexity**
- **4 SSE-related tests** need special attention for Hono's `streamSSE()` 
- Real HTTP SSE connection testing patterns must be preserved
- SSE manager mocking strategies need updates

#### 3. **Error Handling Overhaul**
- Fastify error middleware → Hono error handler patterns
- Error response format consistency must be maintained
- Request ID tracking patterns need adaptation

### What Can Be Removed/Simplified

1. **`sse.test.ts`** - Tests deprecated endpoints, can be removed
2. **Redundant error tests** - Some validation tests overlap
3. **Legacy SSE patterns** - Outdated streaming implementation tests

### Test Coverage Gaps Identified

1. **Authentication tests** - No auth/authorization coverage
2. **File upload tests** - Limited binary data handling
3. **Performance tests** - No load/stress testing
4. **Database integration tests** - Limited data layer testing

## Phase 1: Setup and Core Infrastructure

### 1.1 Dependencies Update

```bash
# Remove Fastify dependencies
pnpm remove fastify fastify-plugin @fastify/cors @fastify/helmet @fastify/multipart @fastify/rate-limit @fastify/static @fastify/swagger @fastify/websocket

# Install Hono and related packages
pnpm install hono @hono/node-server
pnpm install --save-dev @hono/vite-dev-server
```

### 1.2 Core App Structure

**File: `apps/backend/app.ts`**

```typescript
import { Hono } from 'hono'
import { cors } from 'hono/cors'
import { logger } from 'hono/logger'
import { secureHeaders } from 'hono/secure-headers'
import { setupServices } from './services/index.js'
import { registerRoutes } from './routes/index.js'
import { errorHandler } from './middleware/error-handler.js'
import { requestId } from './middleware/request-id.js'

export function createApp() {
  const app = new Hono()

  // Security middleware
  app.use('*', secureHeaders())
  
  // CORS configuration
  app.use('*', cors({
    origin: (origin) => {
      if (process.env.NODE_ENV !== 'production' && origin) {
        const url = new URL(origin)
        if (url.hostname === 'localhost' || url.hostname === '127.0.0.1') {
          return origin
        }
      }
      return process.env.CORS_ORIGIN || true
    },
    credentials: true,
  }))

  // Logging
  app.use('*', logger())
  
  // Request ID middleware
  app.use('*', requestId())

  // Setup services
  setupServices(app)

  // Register routes
  registerRoutes(app)

  // Error handling
  app.onError(errorHandler)

  return app
}
```

### 1.3 Server Entry Point

**File: `apps/backend/index.ts`**

```typescript
import { config } from 'dotenv'
import { serve } from '@hono/node-server'
import { createApp } from './app.js'

config()

const app = createApp()
const port = parseInt(process.env.PORT || '8000', 10)

console.log(`Server starting on port ${port}`)
serve({
  fetch: app.fetch,
  port,
})
```

## Phase 2: Route Migration

### 2.1 Route Structure Conversion

**File: `apps/backend/routes/index.ts`**

```typescript
import { Hono } from 'hono'
import { taskRoutes } from './tasks.js'
import { eventRoutes } from './events.js'
import { repositoryRoutes } from './repositories.js'
import { commandRoutes } from './commands.js'
import { uploadRoutes } from './uploads.js'

const API_PREFIX = '/api/v1'

export function registerRoutes(app: Hono) {
  // Main API info
  app.get(`${API_PREFIX}`, (c) => c.json({
    message: 'Moatless API',
    version: '1.0.0',
    endpoints: {
      health: '/health',
      tasks: `${API_PREFIX}/tasks`,
      repositories: `${API_PREFIX}/repositories`,
      commands: `${API_PREFIX}/commands`,
      events: `${API_PREFIX}/events`,
    },
  }))

  // Register route groups
  app.route(`${API_PREFIX}/tasks`, taskRoutes)
  app.route(`${API_PREFIX}/repositories`, repositoryRoutes)
  app.route(`${API_PREFIX}/events`, eventRoutes)
  app.route(`${API_PREFIX}/uploads`, uploadRoutes)
  app.route(`${API_PREFIX}/commands`, commandRoutes)

  // Health check
  app.get('/health', (c) => c.json({
    status: 'ok',
    timestamp: new Date().toISOString(),
    version: process.env.npm_package_version || '1.0.0',
  }))
}
```

### 2.2 Task Routes Migration

**File: `apps/backend/routes/tasks.ts`**

```typescript
import { Hono } from 'hono'
import { validator } from 'hono/validator'
import { z } from 'zod'
import { TaskNotFoundError } from '@moatless/shared/constants/errors'
import { CreateTaskRequestSchema, SendMessageRequestSchema } from '@moatless/shared/types/api'

const taskRoutes = new Hono()

// Validation middleware
const validateTaskId = validator('param', z.object({ taskId: z.string() }))
const validateCreateTask = validator('json', CreateTaskRequestSchema)
const validateSendMessage = validator('json', SendMessageRequestSchema)

// GET /tasks
taskRoutes.get('/', async (c) => {
  const services = c.get('services')
  const tasks = await services.task.listTasks()
  return c.json(tasks)
})

// POST /tasks
taskRoutes.post('/', validateCreateTask, async (c) => {
  const services = c.get('services')
  const taskData = c.req.valid('json')
  const task = await services.task.createTask(taskData)
  return c.json(task, 201)
})

// GET /tasks/:taskId
taskRoutes.get('/:taskId', validateTaskId, async (c) => {
  const services = c.get('services')
  const { taskId } = c.req.valid('param')
  const task = await services.task.getTask(taskId)
  if (!task) {
    throw new TaskNotFoundError(taskId)
  }
  return c.json(task)
})

export { taskRoutes }
```

## Phase 3: SSE Implementation

### 3.1 Enhanced SSE Manager

**File: `apps/backend/utils/sse-manager.ts`**

```typescript
import { streamSSE } from 'hono/streaming'
import type { Context } from 'hono'

export class HonoSSEManager {
  private connections = new Map<string, Set<Context>>()
  private globalConnections = new Set<Context>()

  // Global event stream
  async createGlobalStream(c: Context) {
    const connectionId = Math.random().toString(36).substr(2, 9)
    
    return streamSSE(c, async (stream) => {
      // Add connection to global set
      this.globalConnections.add(c)
      
      // Send initial connection event
      await stream.writeSSE({
        data: JSON.stringify({
          type: 'connection',
          connectionId,
          timestamp: new Date().toISOString(),
        }),
        event: 'connection',
        id: connectionId,
      })

      // Keep connection alive
      const keepAlive = setInterval(async () => {
        try {
          await stream.writeSSE({
            data: JSON.stringify({ type: 'ping' }),
            event: 'ping',
          })
        } catch (error) {
          clearInterval(keepAlive)
          this.globalConnections.delete(c)
        }
      }, 30000)

      // Handle connection close
      c.req.raw.signal?.addEventListener('abort', () => {
        clearInterval(keepAlive)
        this.globalConnections.delete(c)
      })
    })
  }

  // Task-specific event stream
  async createTaskStream(c: Context, taskId: string) {
    if (!this.connections.has(taskId)) {
      this.connections.set(taskId, new Set())
    }

    const taskConnections = this.connections.get(taskId)!
    
    return streamSSE(c, async (stream) => {
      taskConnections.add(c)

      // Send initial task events
      await stream.writeSSE({
        data: JSON.stringify({
          type: 'task_stream_connected',
          taskId,
          timestamp: new Date().toISOString(),
        }),
        event: 'task_connected',
        id: `${taskId}-${Date.now()}`,
      })

      // Keep connection alive
      const keepAlive = setInterval(async () => {
        try {
          await stream.writeSSE({
            data: JSON.stringify({ type: 'ping' }),
            event: 'ping',
          })
        } catch (error) {
          clearInterval(keepAlive)
          taskConnections.delete(c)
        }
      }, 30000)

      // Handle connection close
      c.req.raw.signal?.addEventListener('abort', () => {
        clearInterval(keepAlive)
        taskConnections.delete(c)
        if (taskConnections.size === 0) {
          this.connections.delete(taskId)
        }
      })
    })
  }
}

export const sseManager = new HonoSSEManager()
```

### 3.2 Event Routes with SSE

**File: `apps/backend/routes/events.ts`**

```typescript
import { Hono } from 'hono'
import { streamSSE } from 'hono/streaming'
import { validator } from 'hono/validator'
import { z } from 'zod'

const eventRoutes = new Hono()

// Global event stream
eventRoutes.get('/stream', async (c) => {
  return streamSSE(c, async (stream) => {
    const connectionId = Math.random().toString(36).substr(2, 9)
    
    // Send connection established event
    await stream.writeSSE({
      data: JSON.stringify({
        type: 'connection_established',
        connectionId,
        timestamp: new Date().toISOString(),
      }),
      event: 'connection',
      id: connectionId,
    })

    // Setup event listener for global events
    const eventHandler = async (event: any) => {
      await stream.writeSSE({
        data: JSON.stringify(event),
        event: event.type,
        id: `${event.id || Date.now()}`,
      })
    }

    // Register for global events
    const services = c.get('services')
    services.event.on('global', eventHandler)

    // Keep connection alive
    const keepAlive = setInterval(async () => {
      try {
        await stream.writeSSE({
          data: JSON.stringify({ type: 'ping' }),
          event: 'ping',
        })
      } catch (error) {
        clearInterval(keepAlive)
        services.event.off('global', eventHandler)
      }
    }, 30000)

    // Handle connection close
    c.req.raw.signal?.addEventListener('abort', () => {
      clearInterval(keepAlive)
      services.event.off('global', eventHandler)
    })
  })
})

// Task-specific event stream
eventRoutes.get('/stream/:taskId', 
  validator('param', z.object({ taskId: z.string() })),
  async (c) => {
    const { taskId } = c.req.valid('param')
    const services = c.get('services')
    
    return streamSSE(c, async (stream) => {
      // Send task stream connected event
      await stream.writeSSE({
        data: JSON.stringify({
          type: 'task_stream_connected',
          taskId,
          timestamp: new Date().toISOString(),
        }),
        event: 'task_connected',
        id: `${taskId}-${Date.now()}`,
      })

      // Setup task-specific event listener
      const taskEventHandler = async (event: any) => {
        await stream.writeSSE({
          data: JSON.stringify(event),
          event: event.type,
          id: `${taskId}-${event.id || Date.now()}`,
        })
      }

      // Register for task events
      services.event.on(`task:${taskId}`, taskEventHandler)

      // Keep connection alive
      const keepAlive = setInterval(async () => {
        try {
          await stream.writeSSE({
            data: JSON.stringify({ type: 'ping' }),
            event: 'ping',
          })
        } catch (error) {
          clearInterval(keepAlive)
          services.event.off(`task:${taskId}`, taskEventHandler)
        }
      }, 30000)

      // Handle connection close
      c.req.raw.signal?.addEventListener('abort', () => {
        clearInterval(keepAlive)
        services.event.off(`task:${taskId}`, taskEventHandler)
      })
    })
  })

export { eventRoutes }
```

## Phase 4: Middleware Migration

### 4.1 Error Handler Middleware

**File: `apps/backend/middleware/error-handler.ts`**

```typescript
import type { ErrorHandler } from 'hono'
import { MoatlessError } from '@moatless/shared/constants/errors'

export const errorHandler: ErrorHandler = (error, c) => {
  const requestId = c.get('requestId')

  if (error instanceof MoatlessError) {
    return c.json({
      error: {
        code: error.code,
        message: error.message,
        details: error.details,
      },
      requestId,
    }, error.statusCode)
  }

  // Handle validation errors
  if (error.name === 'ValidationError') {
    return c.json({
      error: {
        code: 'VALIDATION_ERROR',
        message: 'Request validation failed',
        details: error.message,
      },
      requestId,
    }, 400)
  }

  // Log unexpected errors
  console.error('Unexpected error:', error)

  return c.json({
    error: {
      code: 'INTERNAL_SERVER_ERROR',
      message: 'Internal server error',
    },
    requestId,
  }, 500)
}
```

### 4.2 Request ID Middleware

**File: `apps/backend/middleware/request-id.ts`**

```typescript
import { randomUUID } from 'node:crypto'
import { createMiddleware } from 'hono/factory'

export const requestId = () => {
  return createMiddleware(async (c, next) => {
    const requestId = c.req.header('x-request-id') || randomUUID()
    c.set('requestId', requestId)
    c.res.headers.set('x-request-id', requestId)
    await next()
  })
}
```

## Phase 5: Testing Strategy

### 5.1 Test Helper Updates

**File: `apps/backend/test/helper.ts`**

```typescript
import { Hono } from 'hono'
import { createApp } from '../app.js'

export function createTestApp() {
  return createApp()
}

export async function testRequest(
  app: Hono, 
  method: string, 
  path: string, 
  options: {
    headers?: Record<string, string>
    body?: any
    query?: Record<string, string>
  } = {}
) {
  const url = new URL(`http://localhost${path}`)
  
  // Add query parameters
  if (options.query) {
    Object.entries(options.query).forEach(([key, value]) => {
      url.searchParams.set(key, value)
    })
  }

  const headers = new Headers({
    'Content-Type': 'application/json',
    ...options.headers,
  })

  const request = new Request(url.toString(), {
    method: method.toUpperCase(),
    headers,
    body: options.body ? JSON.stringify(options.body) : undefined,
  })

  return await app.fetch(request)
}

export async function expectJsonResponse(response: Response) {
  expect(response.headers.get('content-type')).toContain('application/json')
  const json = await response.json()
  return json
}

export async function expectSSEResponse(response: Response) {
  expect(response.headers.get('content-type')).toBe('text/event-stream')
  expect(response.headers.get('cache-control')).toBe('no-cache')
  expect(response.headers.get('connection')).toBe('keep-alive')
  return response
}
```

### 5.2 SSE Test Utilities

**File: `apps/backend/test/sse-helper.ts`**

```typescript
export class SSETestClient {
  private reader: ReadableStreamDefaultReader<Uint8Array> | null = null
  private decoder = new TextDecoder()
  private events: any[] = []
  private isConnected = false

  async connect(url: string, signal?: AbortSignal) {
    const response = await fetch(url, {
      headers: { Accept: 'text/event-stream' },
      signal,
    })

    if (!response.ok) {
      throw new Error(`SSE connection failed: ${response.status}`)
    }

    this.reader = response.body!.getReader()
    this.isConnected = true
    return response
  }

  async readEvents(maxEvents = 10, timeout = 5000): Promise<any[]> {
    if (!this.reader) throw new Error('Not connected')

    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        reject(new Error(`Timeout waiting for ${maxEvents} events`))
      }, timeout)

      const readLoop = async () => {
        try {
          let buffer = ''
          
          while (this.events.length < maxEvents && this.isConnected) {
            const { done, value } = await this.reader!.read()
            if (done) break

            buffer += this.decoder.decode(value, { stream: true })
            const lines = buffer.split('\n')
            buffer = lines.pop() || ''

            for (const line of lines) {
              if (line.startsWith('data: ')) {
                try {
                  const data = JSON.parse(line.slice(6))
                  this.events.push(data)
                } catch {
                  // Ignore invalid JSON
                }
              }
            }
          }

          clearTimeout(timer)
          resolve(this.events.slice(0, maxEvents))
        } catch (error) {
          clearTimeout(timer)
          reject(error)
        }
      }

      readLoop()
    })
  }

  disconnect() {
    this.isConnected = false
    this.reader?.cancel()
    this.reader = null
  }
}
```

## Migration Timeline

### Week 1-2: Foundation & Core API
**Focus: Infrastructure + Critical API Tests**
- [ ] Update dependencies and create basic Hono app structure
- [ ] Implement core middleware (CORS, security, logging, error handling)
- [ ] Create Hono test utilities (replace `app.inject()` patterns)
- [ ] **CRITICAL**: Migrate `error-handling.test.ts` (establishes error patterns)
- [ ] **CRITICAL**: Migrate `tasks.test.ts` (420 lines - most complex business logic)
- [ ] Setup service injection pattern for Hono context

### Week 3: Core Features & Repository Management  
**Focus: Repository + Workspace Operations**
- [ ] Convert all route handlers (tasks, repositories, commands, uploads)
- [ ] **CRITICAL**: Migrate `repositories.test.ts` and `repositories-e2e.test.ts`
- [ ] **CRITICAL**: Migrate `workspace-endpoints.test.ts` (file operations)
- [ ] Update validation patterns and service mocking strategies
- [ ] Ensure core CRUD operations work correctly

### Week 4: SSE/Streaming & Events
**Focus: Real-time Features**
- [ ] Implement streamSSE-based event streaming with Hono
- [ ] Create enhanced SSE manager for Hono context
- [ ] **HIGH IMPACT**: Migrate `events-sse-real-server.test.ts` (real HTTP SSE)
- [ ] Migrate remaining 3 event-related test files
- [ ] Test real-time functionality and SSE connection management
- [ ] Performance optimization for streaming

### Week 5: Integration & Final Testing
**Focus: E2E Validation + Cleanup**
- [ ] Migrate integration tests: `agent-lifecycle.test.ts`, `server-lifecycle-e2e.test.ts`
- [ ] Migrate `complete-workflow-e2e.test.ts` (full workflow validation)
- [ ] **CLEANUP**: Remove/simplify `sse.test.ts` (deprecated endpoints)
- [ ] Remove Fastify dependencies and update documentation
- [ ] Comprehensive testing and performance benchmarking
- [ ] Final deployment preparation

### Test Migration Priority Order

#### **Week 1-2 (Foundation)**
1. `error-handling.test.ts` ⭐ - Establishes error patterns
2. `tasks.test.ts` ⭐ - Core business logic (420 lines)

#### **Week 3 (Core Features)**
3. `repositories.test.ts` + `repositories-e2e.test.ts` ⭐ 
4. `workspace-endpoints.test.ts` ⭐

#### **Week 4 (SSE/Events)**
5. `events-sse-real-server.test.ts` 🔄 - Real HTTP SSE
6. `events-global-stream.test.ts` 🔄 - SSE manager mocking  
7. `events-backend-flow.test.ts` 🔄 - Event service integration
8. `events-subscription.test.ts` 🔄 - Subscription patterns

#### **Week 5 (Integration)**
9. `agent-lifecycle.test.ts` 📊 - Agent management
10. `server-lifecycle-e2e.test.ts` 📊 - Server features
11. `complete-workflow-e2e.test.ts` 📊 - Full E2E workflow
12. `sse.test.ts` 🗑️ - Cleanup/remove deprecated tests

## Testing Refactoring

### Key Changes for Hono Testing

1. **Replace `app.inject()` → `app.fetch()`**: Use standard Fetch API
2. **Service Mocking**: Use middleware to inject mock services
3. **Request Construction**: Build proper Request objects
4. **SSE Testing**: Use ReadableStream reader for streaming
5. **Error Format**: Maintain consistent error responses

### Test Categories to Maintain

- **Unit Tests**: Route handlers, validation, CRUD operations
- **Integration Tests**: Service interaction, HTTP requests
- **SSE Tests**: Streaming functionality, connection management
- **Error Tests**: All error scenarios and status codes
- **E2E Tests**: Complete workflows with real server

## Detailed Test Migration Strategies

### Critical Test Patterns to Update

#### 1. **HTTP Testing Pattern Changes**
```typescript
// BEFORE (Fastify)
const response = await app.inject({
  method: 'POST',
  url: '/api/v1/tasks',
  payload: { agentType: 'claude-code', repositoryId: 'test-env', message: 'Hello' }
})

// AFTER (Hono)
const response = await testRequest(app, 'POST', '/api/v1/tasks', {
  body: { agentType: 'claude-code', repositoryId: 'test-env', message: 'Hello' }
})
```

#### 2. **Service Mocking Strategy Changes**
```typescript
// BEFORE (Fastify)
vi.spyOn(app.services.task, 'createTask').mockResolvedValue(mockTaskId)

// AFTER (Hono) 
const mockServices = { task: { createTask: vi.fn().mockResolvedValue(mockTaskId) } }
app.use('*', async (c, next) => {
  c.set('services', mockServices)
  await next()
})
```

#### 3. **SSE Testing Pattern Changes**
```typescript
// BEFORE (Fastify) - Complex manual SSE handling
const sseResponse = await fetch(`${serverUrl}/api/v1/events/stream`, {
  headers: { 'Accept': 'text/event-stream' }
})

// AFTER (Hono) - Using new SSE test utilities
const client = new SSETestClient()
await client.connect(`${serverUrl}/api/v1/events/stream`)
const events = await client.readEvents(1, 3000)
```

### Test File Migration Complexity Matrix

| Test File | Lines | Complexity | Migration Effort | Key Challenges |
|-----------|-------|------------|------------------|----------------|
| `error-handling.test.ts` | 235 | **HIGH** | 2-3 days | Error middleware patterns |
| `tasks.test.ts` | 420 | **VERY HIGH** | 4-5 days | Extensive service mocking |
| `repositories.test.ts` | 150 | **MEDIUM** | 2 days | Repository operations |
| `repositories-e2e.test.ts` | 200 | **HIGH** | 3 days | Real filesystem + git |
| `workspace-endpoints.test.ts` | 300 | **HIGH** | 3-4 days | File operations + commands |
| `events-sse-real-server.test.ts` | 140 | **VERY HIGH** | 3-4 days | Real HTTP SSE connections |
| `events-global-stream.test.ts` | 120 | **HIGH** | 2-3 days | SSE manager mocking |
| `events-backend-flow.test.ts` | 100 | **MEDIUM** | 1-2 days | Event service integration |
| `events-subscription.test.ts` | 80 | **MEDIUM** | 1-2 days | Subscription patterns |
| `agent-lifecycle.test.ts` | 180 | **MEDIUM** | 2-3 days | Agent management |
| `server-lifecycle-e2e.test.ts` | 160 | **MEDIUM** | 2-3 days | Server process management |
| `complete-workflow-e2e.test.ts` | 250 | **HIGH** | 3-4 days | Full E2E with Claude API |
| `sse.test.ts` | 50 | **LOW** | 0.5 days | Remove/simplify deprecated |

**Total Estimated Effort: 28-40 days** (5-8 weeks with parallel work)

### Test Migration Checklist

#### **Week 1-2: Foundation Tests**
- [ ] Create `testRequest()` helper function
- [ ] Create `expectJsonResponse()` utility
- [ ] Create `SSETestClient` class
- [ ] Migrate `error-handling.test.ts`
- [ ] Establish service mocking patterns
- [ ] Migrate `tasks.test.ts` (highest complexity)

#### **Week 3: Core Feature Tests**
- [ ] Migrate `repositories.test.ts`
- [ ] Migrate `repositories-e2e.test.ts`
- [ ] Migrate `workspace-endpoints.test.ts`
- [ ] Validate file operations work correctly

#### **Week 4: SSE/Event Tests**
- [ ] Migrate `events-sse-real-server.test.ts`
- [ ] Update SSE manager mocking for Hono
- [ ] Migrate `events-global-stream.test.ts`
- [ ] Migrate `events-backend-flow.test.ts`
- [ ] Migrate `events-subscription.test.ts`

#### **Week 5: Integration Tests**
- [ ] Migrate `agent-lifecycle.test.ts`
- [ ] Migrate `server-lifecycle-e2e.test.ts`
- [ ] Migrate `complete-workflow-e2e.test.ts`
- [ ] Remove/simplify `sse.test.ts`
- [ ] Final validation and cleanup

### Success Criteria

1. **All 12 critical tests pass** with Hono implementation
2. **Same test coverage percentage** maintained
3. **SSE functionality** works identically to Fastify version
4. **Error responses** maintain exact same format
5. **Performance** meets or exceeds Fastify baseline
6. **No regression** in any existing functionality

## Key Benefits After Migration

1. **Enhanced SSE**: Built-in streaming helpers with better event formatting
2. **Better Performance**: Lighter framework with improved throughput
3. **Modern Standards**: Web API compatibility across runtimes
4. **Simplified Code**: Less boilerplate for streaming and middleware
5. **Future-Proof**: Better alignment with modern JavaScript runtimes

## Troubleshooting Common Issues

### Service Injection
- Ensure services are properly injected into Hono context
- Use middleware pattern for service decoration

### SSE Connection Management
- Handle abort signals properly for connection cleanup
- Implement proper error handling in streaming loops

### Validation Migration
- Update Zod integration with Hono validator middleware
- Maintain error response consistency

### File Upload Handling
- Replace Fastify multipart with Hono body parser
- Update file handling logic for new API patterns

This migration guide provides a structured approach to converting from Fastify to Hono while maintaining all current functionality and improving SSE capabilities.