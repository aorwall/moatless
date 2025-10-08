# Async Error Handling Pattern

## Problem

Async operations fail in various ways:
- Unhandled promise rejections crash the application
- Background tasks fail silently
- Timeouts are not implemented consistently
- Error context is lost in async chains

## Solution

**Use the right error handling pattern based on execution context.**

## Context-Based Patterns

### Pattern 1: Route Handlers

**RULE: Routes ALWAYS use try/catch with handleServiceError**

```typescript
// ✅ CORRECT
app.openapi(route, async (c) => {
  try {
    const result = await service.doSomething()
    return c.json(result)
  } catch (error) {
    handleServiceError(error, 'Operation name')
  }
})

// ❌ WRONG - No error handling
app.openapi(route, async (c) => {
  const result = await service.doSomething()  // ❌ Can throw
  return c.json(result)
})
```

### Pattern 2: Background Tasks

**RULE: Background tasks MUST use .catch() to prevent unhandled rejections**

```typescript
// ✅ CORRECT
const task = (async () => {
  await doLongRunningWork()
})().catch((error) => {
  logger.error('Background task failed', error)
  // Update state, clean up resources
})

this.trackBackgroundTask(taskId, task)

// ❌ WRONG - No .catch()
const task = async () => {
  await doSomethingAsync()  // ❌ Unhandled rejection!
}
task()  // ❌ Fire and forget
```

### Pattern 3: Service Methods

**RULE: Services throw domain errors, routes handle HTTP conversion**

```typescript
// ✅ CORRECT - Service throws
export class TaskService {
  async getTask(taskId: string): Promise<Task> {
    const task = await this.storage.getTask(taskId)

    if (!task) {
      throw new TaskNotFoundError(taskId)  // ✅ Domain error
    }

    return task
  }
}

// ❌ WRONG - Service returns null
async getTask(taskId: string): Promise<Task | null> {
  try {
    return await this.storage.getTask(taskId)
  } catch {
    return null  // ❌ Loses error info
  }
}
```

### Pattern 4: Parallel vs Sequential

**RULE: Use Promise.all for independent operations**

```typescript
// ✅ CORRECT - Parallel independent operations
const [a, b, c] = await Promise.all([
  fetchA(),
  fetchB(),
  fetchC(),
])

// ✅ CORRECT - Sequential dependent operations
const task = await createTask()
const workspace = await createWorkspace(task.taskId)  // Needs task
await startAgent(task.taskId)  // Needs workspace

// ❌ WRONG - Sequential when could be parallel
const a = await fetchA()  // ❌ Waits unnecessarily
const b = await fetchB()  // ❌ Waits unnecessarily
const c = await fetchC()  // ❌ Waits unnecessarily
```

### Pattern 5: Timeouts

**RULE: Use AbortController for timeout and cancellation**

```typescript
// ✅ CORRECT
const controller = new AbortController()
const timeoutId = setTimeout(() => controller.abort(), 30000)

try {
  const result = await fetch(url, { signal: controller.signal })
  clearTimeout(timeoutId)
  return result
} catch (error) {
  clearTimeout(timeoutId)
  if (error.name === 'AbortError') {
    throw new TimeoutError()
  }
  throw error
}

// ❌ WRONG - Race doesn't cancel
Promise.race([
  fetch(url),
  new Promise((_, reject) => setTimeout(reject, 30000))
])
```

### Pattern 6: Unhandled Rejections

**RULE: Set up global handlers in application bootstrap**

```typescript
// apps/backend/src/index.ts
process.on('unhandledRejection', (reason, promise) => {
  if (reason instanceof Error) {
    logger.error('Unhandled Rejection:', {
      error: reason.message,
      stack: reason.stack,
    })
  }

  // Special handling during shutdown
  if (isShuttingDown && reason instanceof TypeError) {
    return  // Ignore certain errors during shutdown
  }
})

process.on('uncaughtException', (error) => {
  logger.error('Uncaught Exception:', error)
  gracefulShutdown('UNCAUGHT_EXCEPTION')
})
```

### Pattern 7: Graceful Shutdown

**RULE: Clean up resources in proper order**

```typescript
const gracefulShutdown = async (signal: string) => {
  if (isShuttingDown) return
  isShuttingDown = true

  const timeout = setTimeout(() => process.exit(1), 30000)

  try {
    // 1. Stop accepting new connections
    server?.close()

    // 2. Close active connections
    await sseManager.gracefulShutdown()

    // 3. Shutdown workspaces
    await workspaceService.gracefulShutdown()

    // 4. Wait for pending operations
    await new Promise(resolve => setTimeout(resolve, 1000))

    clearTimeout(timeout)
    process.exit(0)
  } catch (error) {
    logger.error('Shutdown error:', error)
    process.exit(1)
  }
}
```

## Rules for AI Agents

### RULE 1: Routes Use try/catch
```typescript
// ✅ ALWAYS in routes
app.openapi(route, async (c) => {
  try {
    const result = await service.doSomething()
    return c.json(result)
  } catch (error) {
    handleServiceError(error, 'Context')
  }
})
```

### RULE 2: Background Tasks Use .catch()
```typescript
// ✅ ALWAYS for background tasks
const task = asyncOperation()
  .catch((error) => {
    logger.error('Task failed', error)
    // Clean up
  })
```

### RULE 3: Services Throw Errors
```typescript
// ✅ Services throw typed errors
if (!resource) {
  throw new NotFoundError()
}
```

### RULE 4: Parallel Independent Operations
```typescript
// ✅ Use Promise.all
const [a, b] = await Promise.all([fetchA(), fetchB()])

// ❌ Don't await sequentially
const a = await fetchA()
const b = await fetchB()
```

### RULE 5: Use AbortController for Timeouts
```typescript
// ✅ Create controller and timeout
const controller = new AbortController()
setTimeout(() => controller.abort(), 30000)

// ✅ Pass signal to async operation
fetch(url, { signal: controller.signal })
```

### RULE 6: Never Swallow Errors Silently
```typescript
// ❌ WRONG
try {
  await doSomething()
} catch {
  // Silent failure
}

// ✅ CORRECT - Log or rethrow
try {
  await doSomething()
} catch (error) {
  logger.error('Failed', error as Error)
  throw error
}
```

### RULE 7: Preserve Error Context
```typescript
// ✅ Add context when rethrowing
try {
  await operation()
} catch (error) {
  throw new Error(
    `Failed to process ${id}: ${error.message}`,
    { cause: error }
  )
}

// ✅ Use structured logging
const taskLogger = logger.child({ taskId })
taskLogger.error('Failed', error as Error, { phase: 'setup' })
```

## Quick Decision Matrix

| Context | Pattern | Example |
|---------|---------|---------|
| Route handler | `try/catch` + `handleServiceError` | HTTP error handling |
| Background task | `.catch()` | Fire-and-forget async |
| Service method | `throw` domain errors | Let caller handle |
| Independent ops | `Promise.all()` | Parallel execution |
| Dependent ops | Sequential `await` | Must complete in order |
| Timeout needed | `AbortController` | Cancel long operations |
| Global fallback | `process.on('unhandledRejection')` | Last resort logging |

## Common Anti-Patterns

### ❌ Anti-Pattern 1: Swallowing Errors
```typescript
try {
  return await fetchData()
} catch {
  return null  // ❌ Lost error info
}
```

### ❌ Anti-Pattern 2: Mixing Patterns
```typescript
try {
  await operation()
    .catch(err => console.log(err))  // ❌ Confusing
} catch (error) {
  // Never runs
}
```

### ❌ Anti-Pattern 3: Blocking on Background Work
```typescript
// ❌ WRONG - Blocks
async function sendMessage(msg: string) {
  await agent.execute(msg)  // Waits for completion
  return { status: 'sent' }
}

// ✅ CORRECT - Non-blocking
async function sendMessage(msg: string) {
  agent.execute(msg).catch(err => logger.error(err))
  return { status: 'sent' }  // Returns immediately
}
```

## Testing Async Errors

```typescript
it('should handle background task errors', async () => {
  const mockAgent = {
    execute: vi.fn().mockRejectedValue(new Error('Failed'))
  }

  const service = new TaskService(mockAgent)

  // Should not throw
  await expect(service.sendMessage('test')).resolves.not.toThrow()

  // Verify error was logged
  expect(logger.error).toHaveBeenCalledWith(
    'Background task failed',
    expect.any(Error)
  )
})
```
