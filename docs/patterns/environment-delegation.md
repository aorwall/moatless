# Environment Delegation Pattern

## Problem
Different execution environments (Local, Docker) require different implementations for operations like server management, but consumers need a unified API without knowing environment-specific details.

## Solution
Use delegation pattern where the WorkspaceService checks if environments implement optional methods and delegates to them, falling back to default services when needed.

## Implementation

### 1. Define Optional Methods in Base Environment

```typescript
// base-environment.ts
export abstract class BaseEnvironment {
  // Required methods that all environments must implement
  abstract setup(): Promise<void>
  abstract runCommand(cmd: string): Promise<CommandResult>
  abstract teardown(): Promise<void>
  
  // Optional server management methods
  startServer?(): Promise<ServerStatus>
  stopServer?(): Promise<boolean>
  getServerStatus?(): Promise<ServerStatus>
  getServerLogs?(lines?: number): Promise<string>
  getPreviewPort?(): number | undefined
  proxyPreviewRequest?(request: ProxyRequest): Promise<ProxyResponse>
}
```

### 2. Service Delegation Pattern

```typescript
// workspace.service.ts
class WorkspaceService {
  async startServer(taskId: string): Promise<unknown> {
    const workspace = await this.getOrCreateWorkspace(taskId)
    const environment = workspace.getEnvironment()
    
    // Check if environment implements the method
    if (environment.startServer) {
      logger.info('Delegating server start to environment', {
        taskId,
        environmentType: environment.getType(),
      })
      return environment.startServer()
    }
    
    // Fallback to default service
    if (!this.previewEnvironmentService) {
      throw new WorkspaceError('Preview environment service not initialized', taskId)
    }
    
    return this.previewEnvironmentService.startServer(taskId)
  }
}
```

### 3. Environment-Specific Implementations

#### Local Environment
```typescript
class LocalEnvironment extends BaseEnvironment {
  constructor(
    private previewEnvironmentService: PreviewEnvironmentService
  ) {
    super()
  }
  
  // Delegate to existing service
  async startServer(): Promise<ServerStatus> {
    return this.previewEnvironmentService.startServer(this.taskId)
  }
  
  getPreviewPort(): number | undefined {
    // Direct port access for local environments
    return this.previewEnvironmentService.getServerPort(this.taskId)
  }
}
```

#### Docker Environment
```typescript
class DockerEnvironment extends BaseEnvironment {
  // HTTP-based implementation
  async startServer(): Promise<ServerStatus> {
    const response = await fetch(
      `http://localhost:${this.workerPort}/server/start`,
      {
        method: 'POST',
        body: JSON.stringify({
          serverCommand: this.repositoryConfig.previewConfig.serverStartCommand,
          envVars: this.repositoryConfig.codeExecution.envVars
        })
      }
    )
    
    if (!response.ok) {
      throw new Error(`Failed to start server: ${response.statusText}`)
    }
    
    return response.json()
  }
  
  async proxyPreviewRequest(request: ProxyRequest): Promise<ProxyResponse> {
    // Proxy through worker container
    const response = await fetch(
      `http://localhost:${this.workerPort}/server/proxy${request.path}`,
      {
        method: request.method,
        headers: request.headers,
        body: request.body
      }
    )
    
    return {
      status: response.status,
      headers: response.headers,
      body: await response.arrayBuffer()
    }
  }
  
  getPreviewPort(): undefined {
    // No direct port access in Docker
    return undefined
  }
}
```

### 4. Consumer Usage Pattern

```typescript
// Routes don't need to know about environment types
app.get('/api/v1/tasks/:taskId/proxy/*', async (c) => {
  const workspace = await services.workspace.getOrCreateWorkspace(taskId)
  const serverPort = workspace.getPreviewPort()
  
  if (serverPort) {
    // Local environment - direct proxy
    return proxy(`http://localhost:${serverPort}${path}`)
  } else {
    // Docker environment - delegated proxy
    return workspace.proxyPreviewRequest({
      method: c.req.method,
      path,
      headers: c.req.headers,
      body: c.req.body
    })
  }
})
```

## Benefits

1. **Separation of Concerns**: Each environment handles its own implementation details
2. **Extensibility**: New environments can be added without modifying existing code
3. **Type Safety**: TypeScript ensures optional methods are checked before use
4. **Backward Compatibility**: Fallback to existing services preserves old behavior
5. **Testability**: Each environment can be tested in isolation

## Testing Strategy

### Unit Tests
```typescript
describe('WorkspaceService delegation', () => {
  it('should delegate to environment when method exists', async () => {
    const mockEnv = {
      startServer: vi.fn().mockResolvedValue({ running: true }),
      getType: () => 'docker'
    }
    
    const workspace = { getEnvironment: () => mockEnv }
    vi.mocked(service.getOrCreateWorkspace).mockResolvedValue(workspace)
    
    await service.startServer('task-1')
    
    expect(mockEnv.startServer).toHaveBeenCalled()
    expect(previewService.startServer).not.toHaveBeenCalled()
  })
  
  it('should fallback to preview service when method missing', async () => {
    const mockEnv = { getType: () => 'custom' }
    const workspace = { getEnvironment: () => mockEnv }
    
    await service.startServer('task-1')
    
    expect(previewService.startServer).toHaveBeenCalledWith('task-1')
  })
})
```

## Common Pitfalls

1. **Forgetting to check if method exists**: Always use `if (environment.methodName)` before calling
2. **Inconsistent return types**: Ensure all implementations return the same type
3. **Missing error handling**: Each implementation should handle its specific errors
4. **Tight coupling**: Don't pass environment-specific parameters through the generic API

## When to Use This Pattern

- When different environments need different implementations
- When you want to maintain backward compatibility
- When implementations vary significantly between environments
- When you need to support pluggable environments

## When NOT to Use This Pattern

- When all environments share the same implementation
- When the differences are minor (use configuration instead)
- When it adds unnecessary complexity for simple operations