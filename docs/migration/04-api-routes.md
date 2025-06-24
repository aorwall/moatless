# API Routes Migration Guide

This guide covers the migration of API endpoints from Python FastAPI to TypeScript with Fastify.

## Overview

Phase 4 establishes the REST API endpoints and WebSocket connections, providing the interface between the client and server components with proper validation, error handling, and real-time communication.

## Python Source Files to Migrate

- `/moatless-swarm/swarm/api/tasks.py` - Task API endpoints
- `/moatless-swarm/swarm/api/environments.py` - Environment API endpoints  
- `/moatless-swarm/swarm/api/workspaces.py` - Workspace API endpoints
- `/moatless-swarm/swarm/api/websocket.py` - WebSocket handlers

## Implementation

### 1. Route Registration System

#### server/routes/index.ts
```typescript
import type { FastifyInstance } from 'fastify';
import { registerTaskRoutes } from './tasks.js';
import { registerEnvironmentRoutes } from './environments.js';
import { registerWorkspaceRoutes } from './workspaces.js';

export async function registerRoutes(app: FastifyInstance) {
  // Register API route groups with prefixes
  await app.register(registerTaskRoutes, { prefix: '/api/tasks' });
  await app.register(registerEnvironmentRoutes, { prefix: '/api/environments' });
  await app.register(registerWorkspaceRoutes, { prefix: '/api/workspaces' });
  
  // Catch-all for client-side routing in production
  if (process.env.NODE_ENV === 'production') {
    app.get('/*', async (request, reply) => {
      return reply.sendFile('index.html');
    });
  }
}
```

### 2. Task API Routes

#### server/routes/tasks.ts
```typescript
import type { FastifyInstance } from 'fastify';
import { 
  CreateTaskRequestSchema, 
  TaskResponseSchema,
  SendMessageRequestSchema 
} from '@shared/types/api';
import { TaskNotFoundError } from '@shared/constants/errors.js';

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
  app.get('/:taskId', {
    schema: {
      params: {
        type: 'object',
        properties: {
          taskId: { type: 'string' }
        },
        required: ['taskId']
      }
    }
  }, async (request, reply) => {
    const { taskId } = request.params as { taskId: string };
    const task = await taskService.getTask(taskId);
    
    if (!task) {
      throw new TaskNotFoundError(taskId);
    }
    
    return task;
  });
  
  // List all tasks with pagination
  app.get('/', {
    schema: {
      querystring: {
        type: 'object',
        properties: {
          limit: { type: 'number', minimum: 1, maximum: 100, default: 20 },
          offset: { type: 'number', minimum: 0, default: 0 },
          status: { 
            type: 'string', 
            enum: ['active', 'completed', 'error', 'closed'] 
          }
        }
      }
    }
  }, async (request, reply) => {
    const { limit = 20, offset = 0, status } = request.query as {
      limit?: number;
      offset?: number;
      status?: string;
    };
    
    let tasks = await taskService.listTasks(limit, offset);
    
    // Filter by status if provided
    if (status) {
      tasks = tasks.filter(task => task.status === status);
    }
    
    return {
      tasks,
      pagination: {
        limit,
        offset,
        total: tasks.length // Note: This is simplified, real implementation would need total count
      }
    };
  });
  
  // Send message to task
  app.post('/:taskId/messages', {
    schema: {
      params: {
        type: 'object',
        properties: {
          taskId: { type: 'string' }
        },
        required: ['taskId']
      },
      body: SendMessageRequestSchema
    }
  }, async (request, reply) => {
    const { taskId } = request.params as { taskId: string };
    const { message } = request.body;
    
    await taskService.sendMessage(taskId, message);
    return { status: 'sent', timestamp: new Date().toISOString() };
  });
  
  // Get task events with pagination
  app.get('/:taskId/events', {
    schema: {
      params: {
        type: 'object',
        properties: {
          taskId: { type: 'string' }
        },
        required: ['taskId']
      },
      querystring: {
        type: 'object',
        properties: {
          limit: { type: 'number', minimum: 1, maximum: 1000, default: 100 },
          offset: { type: 'number', minimum: 0, default: 0 },
          type: { type: 'string' }
        }
      }
    }
  }, async (request, reply) => {
    const { taskId } = request.params as { taskId: string };
    const { limit = 100, offset = 0, type } = request.query as {
      limit?: number;
      offset?: number;
      type?: string;
    };
    
    // Verify task exists
    const task = await taskService.getTask(taskId);
    if (!task) {
      throw new TaskNotFoundError(taskId);
    }
    
    const events = await eventService.getEvents(
      taskId,
      limit,
      offset,
      type as any
    );
    
    return {
      events,
      pagination: {
        limit,
        offset,
        hasMore: events.length === limit
      }
    };
  });
  
  // Update task
  app.patch('/:taskId', {
    schema: {
      params: {
        type: 'object',
        properties: {
          taskId: { type: 'string' }
        },
        required: ['taskId']
      },
      body: {
        type: 'object',
        properties: {
          status: { 
            type: 'string', 
            enum: ['active', 'completed', 'error', 'closed'] 
          }
        }
      }
    }
  }, async (request, reply) => {
    const { taskId } = request.params as { taskId: string };
    const updates = request.body as { status?: string };
    
    await taskService.updateTask(taskId, updates);
    return { status: 'updated' };
  });
  
  // Delete task
  app.delete('/:taskId', {
    schema: {
      params: {
        type: 'object',
        properties: {
          taskId: { type: 'string' }
        },
        required: ['taskId']
      }
    }
  }, async (request, reply) => {
    const { taskId } = request.params as { taskId: string };
    
    await taskService.deleteTask(taskId);
    return { status: 'deleted' };
  });
  
  // WebSocket for real-time events
  app.get('/:taskId/events/stream', { 
    websocket: true,
    schema: {
      params: {
        type: 'object',
        properties: {
          taskId: { type: 'string' }
        },
        required: ['taskId']
      }
    }
  }, (connection, request) => {
    const { taskId } = request.params as { taskId: string };
    
    // Verify task exists before setting up subscription
    taskService.getTask(taskId).then(task => {
      if (!task) {
        connection.socket.close(1008, 'Task not found');
        return;
      }
      
      const unsubscribe = eventService.subscribe(taskId, (event) => {
        try {
          connection.socket.send(JSON.stringify(event));
        } catch (error) {
          request.log.error('Failed to send event over websocket', error);
        }
      });
      
      // Send initial connection event
      connection.socket.send(JSON.stringify({
        type: 'connection',
        status: 'connected',
        taskId,
        timestamp: new Date().toISOString()
      }));
      
      connection.socket.on('close', () => {
        unsubscribe();
      });
      
      connection.socket.on('error', (error) => {
        request.log.error('WebSocket error', error);
        unsubscribe();
      });
      
    }).catch(error => {
      request.log.error('Error verifying task for WebSocket', error);
      connection.socket.close(1011, 'Internal server error');
    });
  });
}
```

### 3. Environment API Routes

#### server/routes/environments.ts
```typescript
import type { FastifyInstance } from 'fastify';
import { 
  CreateEnvironmentRequestSchema 
} from '@shared/types/api';
import { EnvironmentNotFoundError } from '@shared/constants/errors.js';

export async function registerEnvironmentRoutes(app: FastifyInstance) {
  const { environmentService } = app.services;
  
  // Create environment
  app.post('/', {
    schema: {
      body: CreateEnvironmentRequestSchema
    }
  }, async (request, reply) => {
    const environmentData = request.body;
    const environment = await environmentService.createEnvironment(environmentData);
    return environment;
  });
  
  // List environments
  app.get('/', async (request, reply) => {
    const environments = await environmentService.listEnvironments();
    return { environments };
  });
  
  // Get environment by ID
  app.get('/:environmentId', {
    schema: {
      params: {
        type: 'object',
        properties: {
          environmentId: { type: 'string' }
        },
        required: ['environmentId']
      }
    }
  }, async (request, reply) => {
    const { environmentId } = request.params as { environmentId: string };
    const environment = await environmentService.getEnvironment(environmentId);
    
    if (!environment) {
      throw new EnvironmentNotFoundError(environmentId);
    }
    
    return environment;
  });
  
  // Update environment
  app.patch('/:environmentId', {
    schema: {
      params: {
        type: 'object',
        properties: {
          environmentId: { type: 'string' }
        },
        required: ['environmentId']
      },
      body: {
        type: 'object',
        properties: {
          name: { type: 'string' },
          description: { type: 'string' }
        }
      }
    }
  }, async (request, reply) => {
    const { environmentId } = request.params as { environmentId: string };
    const updates = request.body as { name?: string; description?: string };
    
    const updatedEnvironment = await environmentService.updateEnvironment(environmentId, updates);
    return updatedEnvironment;
  });
  
  // Delete environment
  app.delete('/:environmentId', {
    schema: {
      params: {
        type: 'object',
        properties: {
          environmentId: { type: 'string' }
        },
        required: ['environmentId']
      }
    }
  }, async (request, reply) => {
    const { environmentId } = request.params as { environmentId: string };
    
    await environmentService.deleteEnvironment(environmentId);
    return { status: 'deleted' };
  });
  
  // Search files in environment
  app.post('/:environmentId/search', {
    schema: {
      params: {
        type: 'object',
        properties: {
          environmentId: { type: 'string' }
        },
        required: ['environmentId']
      },
      body: {
        type: 'object',
        properties: {
          query: { type: 'string', minLength: 1 },
          filePattern: { type: 'string' },
          maxResults: { type: 'number', minimum: 1, maximum: 1000, default: 100 }
        },
        required: ['query']
      }
    }
  }, async (request, reply) => {
    const { environmentId } = request.params as { environmentId: string };
    const searchRequest = request.body as {
      query: string;
      filePattern?: string;
      maxResults?: number;
    };
    
    const results = await environmentService.searchFiles({
      environmentId,
      ...searchRequest
    });
    
    return results;
  });
}
```

### 4. Workspace API Routes

#### server/routes/workspaces.ts
```typescript
import type { FastifyInstance } from 'fastify';
import { WorkspaceError } from '@shared/constants/errors.js';

export async function registerWorkspaceRoutes(app: FastifyInstance) {
  const { workspaceService, taskService } = app.services;
  
  // Get workspace by task ID
  app.get('/:taskId', {
    schema: {
      params: {
        type: 'object',
        properties: {
          taskId: { type: 'string' }
        },
        required: ['taskId']
      }
    }
  }, async (request, reply) => {
    const { taskId } = request.params as { taskId: string };
    const workspace = await workspaceService.getWorkspace(taskId);
    
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId);
    }
    
    return workspace;
  });
  
  // Get workspace status
  app.get('/:taskId/status', {
    schema: {
      params: {
        type: 'object',
        properties: {
          taskId: { type: 'string' }
        },
        required: ['taskId']
      }
    }
  }, async (request, reply) => {
    const { taskId } = request.params as { taskId: string };
    const workspace = await workspaceService.getWorkspace(taskId);
    
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId);
    }
    
    return {
      status: workspace.status,
      type: workspace.type,
      createdAt: workspace.createdAt,
      error: workspace.error
    };
  });
  
  // Cleanup workspace
  app.delete('/:taskId', {
    schema: {
      params: {
        type: 'object',
        properties: {
          taskId: { type: 'string' }
        },
        required: ['taskId']
      }
    }
  }, async (request, reply) => {
    const { taskId } = request.params as { taskId: string };
    
    await workspaceService.cleanupWorkspace(taskId);
    return { status: 'cleaned' };
  });
  
  // Get workspace files (for local workspaces)
  app.get('/:taskId/files', {
    schema: {
      params: {
        type: 'object',
        properties: {
          taskId: { type: 'string' }
        },
        required: ['taskId']
      },
      querystring: {
        type: 'object',
        properties: {
          path: { type: 'string' },
          recursive: { type: 'boolean', default: false }
        }
      }
    }
  }, async (request, reply) => {
    const { taskId } = request.params as { taskId: string };
    const { path = '.', recursive = false } = request.query as {
      path?: string;
      recursive?: boolean;
    };
    
    const files = await workspaceService.listFiles(taskId, path, recursive);
    return { files };
  });
  
  // Read workspace file
  app.get('/:taskId/files/*', {
    schema: {
      params: {
        type: 'object',
        properties: {
          taskId: { type: 'string' },
          '*': { type: 'string' }
        },
        required: ['taskId', '*']
      }
    }
  }, async (request, reply) => {
    const { taskId } = request.params as { taskId: string; '*': string };
    const filePath = request.params['*'];
    
    const content = await workspaceService.readFile(taskId, filePath);
    
    reply.type('text/plain');
    return content;
  });
  
  // Write workspace file
  app.put('/:taskId/files/*', {
    schema: {
      params: {
        type: 'object',
        properties: {
          taskId: { type: 'string' },
          '*': { type: 'string' }
        },
        required: ['taskId', '*']
      },
      body: {
        type: 'object',
        properties: {
          content: { type: 'string' }
        },
        required: ['content']
      }
    }
  }, async (request, reply) => {
    const { taskId } = request.params as { taskId: string; '*': string };
    const filePath = request.params['*'];
    const { content } = request.body as { content: string };
    
    await workspaceService.writeFile(taskId, filePath, content);
    return { status: 'written' };
  });
  
  // Get git diff for workspace
  app.get('/:taskId/diff', {
    schema: {
      params: {
        type: 'object',
        properties: {
          taskId: { type: 'string' }
        },
        required: ['taskId']
      }
    }
  }, async (request, reply) => {
    const { taskId } = request.params as { taskId: string };
    
    const diff = await workspaceService.getGitDiff(taskId);
    return diff;
  });
}
```

### 5. Request/Response Validation

#### server/plugins/validation.ts
```typescript
import type { FastifyInstance } from 'fastify';
import { ValidationError } from '@shared/constants/errors.js';

export async function setupValidation(app: FastifyInstance) {
  // Custom schema compiler for Zod integration
  app.setValidatorCompiler(({ schema }) => {
    return (data) => {
      try {
        if ('safeParse' in schema && typeof schema.safeParse === 'function') {
          // Zod schema
          const result = schema.safeParse(data);
          if (result.success) {
            return { value: result.data };
          } else {
            return { error: new ValidationError('Validation failed', result.error.issues) };
          }
        }
        // Fallback to default validation
        return { value: data };
      } catch (error) {
        return { error };
      }
    };
  });
  
  // Add global request validation
  app.addHook('preValidation', async (request, reply) => {
    // Add request ID for tracing
    if (!request.headers['x-request-id']) {
      request.headers['x-request-id'] = crypto.randomUUID();
    }
  });
}
```

### 6. Rate Limiting

#### server/plugins/rate-limit.ts
```typescript
import type { FastifyInstance } from 'fastify';
import rateLimit from '@fastify/rate-limit';

export async function setupRateLimit(app: FastifyInstance) {
  await app.register(rateLimit, {
    max: 100, // 100 requests
    timeWindow: '1 minute', // per minute
    errorResponseBuilder: () => ({
      error: {
        code: 'RATE_LIMIT_EXCEEDED',
        message: 'Rate limit exceeded. Please try again later.'
      }
    })
  });
  
  // More strict rate limiting for expensive operations
  app.register(async (fastify) => {
    await fastify.register(rateLimit, {
      max: 10,
      timeWindow: '1 minute'
    });
    
    fastify.post('/api/tasks', async (request, reply) => {
      // This endpoint now has stricter rate limiting
    });
  });
}
```

## Testing Strategy

### API Integration Tests

#### server/routes/__tests__/tasks.test.ts
```typescript
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { build } from '../../../test/helper.js';
import type { FastifyInstance } from 'fastify';

describe('Task Routes', () => {
  let app: FastifyInstance;
  
  beforeEach(async () => {
    app = await build();
  });
  
  afterEach(async () => {
    await app.close();
  });
  
  describe('POST /api/tasks', () => {
    it('should create a new task', async () => {
      const response = await app.inject({
        method: 'POST',
        url: '/api/tasks',
        payload: {
          agentType: 'claude-code',
          environmentId: 'test-env',
          message: 'Hello world'
        }
      });
      
      expect(response.statusCode).toBe(200);
      const body = JSON.parse(response.body);
      expect(body.taskId).toBeDefined();
      expect(body.status).toBe('active');
    });
    
    it('should validate request body', async () => {
      const response = await app.inject({
        method: 'POST',
        url: '/api/tasks',
        payload: {
          // Missing required fields
        }
      });
      
      expect(response.statusCode).toBe(400);
    });
  });
  
  describe('GET /api/tasks/:taskId', () => {
    it('should return task details', async () => {
      // First create a task
      const createResponse = await app.inject({
        method: 'POST',
        url: '/api/tasks',
        payload: {
          agentType: 'claude-code',
          environmentId: 'test-env',
          message: 'Hello world'
        }
      });
      
      const { taskId } = JSON.parse(createResponse.body);
      
      // Then get task details
      const response = await app.inject({
        method: 'GET',
        url: `/api/tasks/${taskId}`
      });
      
      expect(response.statusCode).toBe(200);
      const task = JSON.parse(response.body);
      expect(task.taskId).toBe(taskId);
    });
    
    it('should return 404 for non-existent task', async () => {
      const response = await app.inject({
        method: 'GET',
        url: '/api/tasks/non-existent-id'
      });
      
      expect(response.statusCode).toBe(404);
    });
  });
});
```

### WebSocket Integration Tests

#### server/routes/__tests__/websocket.test.ts
```typescript
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { build } from '../../../test/helper.js';
import type { FastifyInstance } from 'fastify';
import WebSocket from 'ws';

describe('WebSocket Events', () => {
  let app: FastifyInstance;
  
  beforeEach(async () => {
    app = await build();
    await app.listen({ port: 0 });
  });
  
  afterEach(async () => {
    await app.close();
  });
  
  it('should stream task events', async () => {
    // Create a task first
    const createResponse = await app.inject({
      method: 'POST',
      url: '/api/tasks',
      payload: {
        agentType: 'claude-code',
        environmentId: 'test-env',
        message: 'Hello world'
      }
    });
    
    const { taskId } = JSON.parse(createResponse.body);
    
    // Connect to WebSocket
    const address = app.server.address();
    const port = typeof address === 'string' ? address : address?.port;
    const ws = new WebSocket(`ws://localhost:${port}/api/tasks/${taskId}/events/stream`);
    
    const messages: any[] = [];
    
    ws.on('message', (data) => {
      messages.push(JSON.parse(data.toString()));
    });
    
    await new Promise((resolve) => {
      ws.on('open', resolve);
    });
    
    // Send a message to trigger events
    await app.inject({
      method: 'POST',
      url: `/api/tasks/${taskId}/messages`,
      payload: { message: 'Test message' }
    });
    
    // Wait for events
    await new Promise(resolve => setTimeout(resolve, 100));
    
    expect(messages.length).toBeGreaterThan(0);
    expect(messages.some(msg => msg.type === 'input')).toBe(true);
    
    ws.close();
  });
});
```

## Migration Checklist

- [ ] Set up route registration system
- [ ] Implement task API endpoints with validation
- [ ] Create environment API endpoints
- [ ] Add workspace API endpoints
- [ ] Implement WebSocket event streaming
- [ ] Add request/response schema validation
- [ ] Create error handling middleware
- [ ] Add rate limiting protection
- [ ] Implement pagination for list endpoints
- [ ] Add API documentation with schemas
- [ ] Write comprehensive API tests
- [ ] Add WebSocket integration tests
- [ ] Test error handling scenarios
- [ ] Validate against Python API behavior

## Best Practices

### 1. Error Handling
- Use consistent error response format
- Provide meaningful error codes and messages
- Log errors with request context
- Handle validation errors gracefully

### 2. Validation
- Validate all input at API boundaries
- Use Zod schemas for runtime validation
- Provide clear validation error messages
- Sanitize file paths and user input

### 3. Performance
- Implement pagination for large datasets
- Use appropriate rate limiting
- Cache frequently accessed data
- Optimize database queries

### 4. Security
- Validate all user inputs
- Implement proper CORS configuration
- Use helmet for security headers
- Sanitize file operations

## Next Steps

1. Review [Environment Management Migration](./05-environment-management.md)
2. Implement comprehensive API documentation
3. Add monitoring and metrics collection
4. Set up automated API testing
5. Configure security scanning and validation