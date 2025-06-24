# Client Integration & Production Build Migration Guide

This guide covers the integration of the React client with the TypeScript server and production build configuration for monolithic deployment.

## Overview

Phase 10 focuses on integrating the existing React client application with the new TypeScript server, setting up production builds, and configuring the monolithic deployment structure for NPX distribution.

## Implementation

### 1. Client Build Integration

#### client/vite.config.ts
```typescript
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tsconfigPaths from 'vite-tsconfig-paths';
import tailwindcss from '@tailwindcss/vite';
import { TanStackRouterVite } from '@tanstack/router-plugin/vite';
import path from 'path';

export default defineConfig(({ mode }) => {
  const isDevelopment = mode === 'development';
  const isProduction = mode === 'production';
  
  return {
    plugins: [
      react(),
      tsconfigPaths(),
      tailwindcss(),
      TanStackRouterVite()
    ],
    
    resolve: {
      alias: {
        '@': path.resolve(__dirname, './src'),
        '@shared': path.resolve(__dirname, '../shared')
      }
    },
    
    server: {
      port: 3000,
      host: true,
      proxy: {
        '/api': {
          target: 'http://localhost:8000',
          changeOrigin: true,
          secure: false
        },
        '/health': {
          target: 'http://localhost:8000',
          changeOrigin: true,
          secure: false
        }
      }
    },
    
    build: {
      outDir: 'dist',
      sourcemap: isProduction ? false : true,
      minify: isProduction ? 'esbuild' : false,
      
      rollupOptions: {
        output: {
          manualChunks: {
            vendor: ['react', 'react-dom'],
            tanstack: ['@tanstack/react-query', '@tanstack/react-router'],
            ui: ['@radix-ui/react-alert-dialog', '@radix-ui/react-checkbox'],
          }
        }
      },
      
      // Optimize for production
      chunkSizeWarningLimit: 1000
    },
    
    optimizeDeps: {
      include: [
        'react',
        'react-dom',
        '@tanstack/react-query',
        '@tanstack/react-router'
      ]
    },
    
    define: {
      __DEV__: isDevelopment,
      __PROD__: isProduction
    }
  };
});
```

### 2. API Client Integration

#### client/src/lib/api/client.ts
```typescript
import { z } from 'zod';
import type { 
  TaskMetadata, 
  Environment, 
  CreateTaskRequest, 
  CreateEnvironmentRequest 
} from '@shared/types';

export class ApiError extends Error {
  constructor(
    message: string,
    public statusCode: number,
    public code?: string,
    public details?: any
  ) {
    super(message);
    this.name = 'ApiError';
  }
}

export interface ApiResponse<T = any> {
  data?: T;
  error?: {
    code: string;
    message: string;
    details?: any;
  };
}

class ApiClient {
  private baseUrl: string;
  
  constructor() {
    this.baseUrl = import.meta.env.VITE_API_URL || '/api';
  }
  
  private async request<T = any>(
    endpoint: string,
    options: RequestInit = {}
  ): Promise<T> {
    const url = `${this.baseUrl}${endpoint}`;
    
    const config: RequestInit = {
      headers: {
        'Content-Type': 'application/json',
        ...options.headers
      },
      ...options
    };
    
    // Add request ID for tracing
    const requestId = crypto.randomUUID();
    config.headers = {
      ...config.headers,
      'X-Request-ID': requestId
    };
    
    try {
      const response = await fetch(url, config);
      
      if (!response.ok) {
        let errorData;
        try {
          errorData = await response.json();
        } catch {
          errorData = { message: response.statusText };
        }
        
        throw new ApiError(
          errorData.error?.message || errorData.message || 'Request failed',
          response.status,
          errorData.error?.code,
          errorData.error?.details
        );
      }
      
      // Handle empty responses
      if (response.status === 204) {
        return {} as T;
      }
      
      const data = await response.json();
      return data;
      
    } catch (error) {
      if (error instanceof ApiError) {
        throw error;
      }
      
      // Network or other errors
      throw new ApiError(
        error instanceof Error ? error.message : 'Network error',
        0
      );
    }
  }
  
  async get<T = any>(endpoint: string): Promise<T> {
    return this.request<T>(endpoint, { method: 'GET' });
  }
  
  async post<T = any>(endpoint: string, data?: any): Promise<T> {
    return this.request<T>(endpoint, {
      method: 'POST',
      body: data ? JSON.stringify(data) : undefined
    });
  }
  
  async put<T = any>(endpoint: string, data?: any): Promise<T> {
    return this.request<T>(endpoint, {
      method: 'PUT',
      body: data ? JSON.stringify(data) : undefined
    });
  }
  
  async patch<T = any>(endpoint: string, data?: any): Promise<T> {
    return this.request<T>(endpoint, {
      method: 'PATCH',
      body: data ? JSON.stringify(data) : undefined
    });
  }
  
  async delete<T = any>(endpoint: string): Promise<T> {
    return this.request<T>(endpoint, { method: 'DELETE' });
  }
  
  // Typed API methods
  
  // Tasks
  async getTasks(limit?: number, offset?: number): Promise<{
    tasks: TaskMetadata[];
    pagination: {
      limit: number;
      offset: number;
      total: number;
    };
  }> {
    const params = new URLSearchParams();
    if (limit) params.set('limit', limit.toString());
    if (offset) params.set('offset', offset.toString());
    
    return this.get(`/tasks?${params.toString()}`);
  }
  
  async getTask(taskId: string): Promise<TaskMetadata> {
    return this.get(`/tasks/${taskId}`);
  }
  
  async createTask(request: CreateTaskRequest): Promise<{ taskId: string; status: string }> {
    return this.post('/tasks', request);
  }
  
  async sendMessage(taskId: string, message: string): Promise<{ status: string }> {
    return this.post(`/tasks/${taskId}/messages`, { message });
  }
  
  async getTaskEvents(
    taskId: string, 
    limit?: number, 
    offset?: number
  ): Promise<{
    events: any[];
    pagination: {
      limit: number;
      offset: number;
      hasMore: boolean;
    };
  }> {
    const params = new URLSearchParams();
    if (limit) params.set('limit', limit.toString());
    if (offset) params.set('offset', offset.toString());
    
    return this.get(`/tasks/${taskId}/events?${params.toString()}`);
  }
  
  async deleteTask(taskId: string): Promise<{ status: string }> {
    return this.delete(`/tasks/${taskId}`);
  }
  
  // Environments
  async getEnvironments(): Promise<{ environments: Environment[] }> {
    return this.get('/environments');
  }
  
  async getEnvironment(environmentId: string): Promise<Environment> {
    return this.get(`/environments/${environmentId}`);
  }
  
  async createEnvironment(request: CreateEnvironmentRequest): Promise<Environment> {
    return this.post('/environments', request);
  }
  
  async updateEnvironment(
    environmentId: string, 
    updates: { name?: string; description?: string }
  ): Promise<Environment> {
    return this.patch(`/environments/${environmentId}`, updates);
  }
  
  async deleteEnvironment(environmentId: string): Promise<{ status: string }> {
    return this.delete(`/environments/${environmentId}`);
  }
  
  // Workspaces
  async getWorkspace(taskId: string): Promise<any> {
    return this.get(`/workspaces/${taskId}`);
  }
  
  async getWorkspaceStatus(taskId: string): Promise<{
    status: string;
    type: string;
    createdAt: string;
    error?: string;
  }> {
    return this.get(`/workspaces/${taskId}/status`);
  }
  
  async getWorkspaceFiles(taskId: string, path?: string): Promise<{ files: string[] }> {
    const params = new URLSearchParams();
    if (path) params.set('path', path);
    
    return this.get(`/workspaces/${taskId}/files?${params.toString()}`);
  }
  
  async readWorkspaceFile(taskId: string, filePath: string): Promise<string> {
    const response = await fetch(`${this.baseUrl}/workspaces/${taskId}/files/${filePath}`);
    return response.text();
  }
  
  async writeWorkspaceFile(
    taskId: string, 
    filePath: string, 
    content: string
  ): Promise<{ status: string }> {
    return this.put(`/workspaces/${taskId}/files/${filePath}`, { content });
  }
  
  async getGitDiff(taskId: string): Promise<any> {
    return this.get(`/workspaces/${taskId}/diff`);
  }
  
  // Health
  async getHealth(): Promise<{
    status: string;
    checks: any[];
    timestamp: string;
    uptime: number;
    version: string;
  }> {
    const response = await fetch('/health');
    return response.json();
  }
}

export const apiClient = new ApiClient();
```

### 3. WebSocket Integration

#### client/src/lib/api/websocket.ts
```typescript
import { useEffect, useRef, useCallback } from 'react';
import type { Event } from '@shared/types';

export interface WebSocketConfig {
  url: string;
  reconnectDelay?: number;
  maxReconnectAttempts?: number;
  pingInterval?: number;
}

export interface WebSocketState {
  connected: boolean;
  connecting: boolean;
  error: string | null;
  reconnectAttempts: number;
}

export function useTaskWebSocket(
  taskId: string,
  onEvent: (event: Event) => void,
  enabled: boolean = true
) {
  const wsRef = useRef<WebSocket | null>(null);
  const reconnectTimeoutRef = useRef<NodeJS.Timeout | null>(null);
  const reconnectAttemptsRef = useRef(0);
  const maxReconnectAttempts = 5;
  const reconnectDelay = 3000;
  
  const [state, setState] = useState<WebSocketState>({
    connected: false,
    connecting: false,
    error: null,
    reconnectAttempts: 0
  });
  
  const connect = useCallback(() => {
    if (!enabled || !taskId) return;
    
    setState(prev => ({ ...prev, connecting: true, error: null }));
    
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const host = window.location.host;
    const wsUrl = `${protocol}//${host}/api/tasks/${taskId}/events/stream`;
    
    try {
      const ws = new WebSocket(wsUrl);
      wsRef.current = ws;
      
      ws.onopen = () => {
        setState(prev => ({
          ...prev,
          connected: true,
          connecting: false,
          error: null,
          reconnectAttempts: 0
        }));
        reconnectAttemptsRef.current = 0;
      };
      
      ws.onmessage = (event) => {
        try {
          const data = JSON.parse(event.data);
          onEvent(data);
        } catch (error) {
          console.error('Failed to parse WebSocket message:', error);
        }
      };
      
      ws.onclose = (event) => {
        setState(prev => ({
          ...prev,
          connected: false,
          connecting: false
        }));
        
        wsRef.current = null;
        
        // Attempt reconnection if not manually closed
        if (event.code !== 1000 && reconnectAttemptsRef.current < maxReconnectAttempts) {
          reconnectAttemptsRef.current++;
          
          setState(prev => ({
            ...prev,
            reconnectAttempts: reconnectAttemptsRef.current
          }));
          
          reconnectTimeoutRef.current = setTimeout(() => {
            connect();
          }, reconnectDelay * reconnectAttemptsRef.current);
        }
      };
      
      ws.onerror = () => {
        setState(prev => ({
          ...prev,
          error: 'WebSocket connection failed',
          connecting: false
        }));
      };
      
    } catch (error) {
      setState(prev => ({
        ...prev,
        error: error instanceof Error ? error.message : 'Connection failed',
        connecting: false
      }));
    }
  }, [taskId, enabled, onEvent]);
  
  const disconnect = useCallback(() => {
    if (reconnectTimeoutRef.current) {
      clearTimeout(reconnectTimeoutRef.current);
      reconnectTimeoutRef.current = null;
    }
    
    if (wsRef.current) {
      wsRef.current.close(1000, 'Manual disconnect');
      wsRef.current = null;
    }
    
    setState({
      connected: false,
      connecting: false,
      error: null,
      reconnectAttempts: 0
    });
  }, []);
  
  useEffect(() => {
    if (enabled) {
      connect();
    } else {
      disconnect();
    }
    
    return disconnect;
  }, [connect, disconnect, enabled]);
  
  return {
    ...state,
    connect,
    disconnect
  };
}
```

### 4. Production Build Configuration

#### Root package.json updates
```json
{
  "scripts": {
    "build": "pnpm run build:shared && pnpm run build:server && pnpm run build:client",
    "build:shared": "tsc -p shared/tsconfig.json",
    "build:server": "tsc -p server/tsconfig.json",
    "build:client": "cd client && pnpm build",
    "build:production": "NODE_ENV=production pnpm run build",
    "start": "node dist/server/index.js",
    "dev": "concurrently \"pnpm:dev:*\"",
    "dev:server": "tsx watch server/index.ts",
    "dev:client": "cd client && pnpm dev",
    "package": "pnpm run build:production && pnpm pack",
    "prepack": "pnpm run build:production"
  },
  "files": [
    "bin/",
    "dist/",
    "client/dist/",
    "shared/",
    "package.json",
    "README.md"
  ]
}
```

### 5. Static File Serving Integration

#### server/app.ts updates
```typescript
import type { FastifyInstance } from 'fastify';
import cors from '@fastify/cors';
import staticPlugin from '@fastify/static';
import websocket from '@fastify/websocket';
import helmet from '@fastify/helmet';
import path from 'path';
import { fileURLToPath } from 'url';
import { promises as fs } from 'fs';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

export async function setupApp(app: FastifyInstance) {
  // Security headers
  await app.register(helmet, {
    contentSecurityPolicy: {
      directives: {
        defaultSrc: ["'self'"],
        styleSrc: ["'self'", "'unsafe-inline'"],
        scriptSrc: ["'self'"],
        imgSrc: ["'self'", "data:", "https:"],
        connectSrc: ["'self'", "ws:", "wss:"],
        fontSrc: ["'self'"],
        objectSrc: ["'none'"],
        mediaSrc: ["'self'"],
        frameSrc: ["'none'"]
      }
    },
    crossOriginEmbedderPolicy: false
  });
  
  // CORS configuration
  await app.register(cors, {
    origin: (origin, callback) => {
      // Allow requests with no origin (mobile apps, curl, etc.)
      if (!origin) return callback(null, true);
      
      // In development, allow any origin
      if (process.env.NODE_ENV === 'development') {
        return callback(null, true);
      }
      
      // In production, check against allowed origins
      const allowedOrigins = (process.env.CORS_ORIGINS || '').split(',');
      const isAllowed = allowedOrigins.includes(origin);
      callback(null, isAllowed);
    },
    credentials: true
  });
  
  // WebSocket support
  await app.register(websocket);
  
  // Serve client build in production
  if (process.env.NODE_ENV === 'production') {
    const clientDistPath = path.join(__dirname, '../../client/dist');
    
    // Check if client build exists
    try {
      await fs.access(clientDistPath);
      
      // Serve static files
      await app.register(staticPlugin, {
        root: clientDistPath,
        prefix: '/',
        decorateReply: false
      });
      
      // SPA fallback - serve index.html for all non-API routes
      app.setNotFoundHandler(async (request, reply) => {
        // Skip API routes
        if (request.url.startsWith('/api') || request.url.startsWith('/health')) {
          return reply.status(404).send({ error: 'Not Found' });
        }
        
        // Serve index.html for client-side routing
        const indexPath = path.join(clientDistPath, 'index.html');
        return reply.type('text/html').sendFile('index.html');
      });
      
    } catch (error) {
      app.log.warn('Client build not found, serving API only');
    }
  }
  
  // ... rest of app setup
}
```

### 6. Environment Configuration

#### .env.production
```bash
# Production Configuration
NODE_ENV=production
PORT=8000
HOST=0.0.0.0

# Logging
LOG_LEVEL=info
LOG_FILE=./logs/moatless.log

# CORS
CORS_ORIGINS=https://moatless.ai,https://app.moatless.ai

# Storage
STORAGE_TYPE=file
STORAGE_PATH=./data

# Performance
PERFORMANCE_MONITORING=true
PERFORMANCE_METRICS_INTERVAL=60000

# Health Checks
HEALTH_ENABLED=true
HEALTH_CHECK_INTERVAL=30000

# Security
HELMET_ENABLED=true
RATE_LIMIT_ENABLED=true
```

### 7. Docker Configuration

#### Dockerfile
```dockerfile
FROM node:22-alpine AS base

# Install pnpm
RUN npm install -g pnpm@10.7.1

WORKDIR /app

# Copy package files
COPY package.json pnpm-lock.yaml ./
COPY client/package.json ./client/
COPY shared/package.json ./shared/
COPY server/package.json ./server/

# Install dependencies
RUN pnpm install --frozen-lockfile

# Copy source code
COPY . .

# Build application
RUN pnpm run build:production

# Production stage
FROM node:22-alpine AS production

# Install pnpm
RUN npm install -g pnpm@10.7.1

WORKDIR /app

# Copy package files
COPY package.json pnpm-lock.yaml ./
COPY client/package.json ./client/
COPY shared/package.json ./shared/
COPY server/package.json ./server/

# Install production dependencies only
RUN pnpm install --frozen-lockfile --prod

# Copy built application
COPY --from=base /app/dist ./dist
COPY --from=base /app/client/dist ./client/dist
COPY --from=base /app/shared ./shared
COPY --from=base /app/bin ./bin

# Create non-root user
RUN addgroup -g 1001 -S nodejs
RUN adduser -S moatless -u 1001

# Create data and logs directories
RUN mkdir -p /app/data /app/logs /app/workspaces /app/clones
RUN chown -R moatless:nodejs /app

USER moatless

# Expose port
EXPOSE 8000

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD curl -f http://localhost:8000/health || exit 1

# Start application
CMD ["node", "dist/server/index.js"]
```

#### docker-compose.yml
```yaml
version: '3.8'

services:
  moatless:
    build: .
    ports:
      - "8000:8000"
    environment:
      - NODE_ENV=production
      - PORT=8000
      - HOST=0.0.0.0
      - LOG_LEVEL=info
      - STORAGE_PATH=/app/data
      - PERFORMANCE_MONITORING=true
    volumes:
      - moatless_data:/app/data
      - moatless_logs:/app/logs
      - moatless_workspaces:/app/workspaces
      - moatless_clones:/app/clones
    restart: unless-stopped
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8000/health"]
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 40s

volumes:
  moatless_data:
  moatless_logs:
  moatless_workspaces:
  moatless_clones:
```

### 8. NPX Distribution Setup

#### bin/cli.js updates
```javascript
#!/usr/bin/env node

import { spawn } from 'child_process';
import { fileURLToPath } from 'url';
import { dirname, join } from 'path';
import { promises as fs } from 'fs';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const rootDir = join(__dirname, '..');

// Parse command line arguments
const args = process.argv.slice(2);
const command = args[0] || 'start';

// Environment setup
process.env.NODE_ENV = process.env.NODE_ENV || 'production';

// Check if we're in development
const isDev = process.env.NODE_ENV === 'development';

switch (command) {
  case 'start':
    await startServer();
    break;
  case 'dev':
    if (!isDev) {
      console.error('Development mode not available in production build');
      process.exit(1);
    }
    await startDevelopment();
    break;
  case 'health':
    await checkHealth();
    break;
  case 'version':
    await showVersion();
    break;
  case 'help':
    showHelp();
    break;
  default:
    console.log(`Unknown command: ${command}`);
    showHelp();
    process.exit(1);
}

async function startServer() {
  const serverPath = join(rootDir, 'dist', 'server', 'index.js');
  
  try {
    await fs.access(serverPath);
  } catch {
    console.error('Server build not found. Please ensure the package is properly installed.');
    process.exit(1);
  }
  
  console.log('Starting Moatless server...');
  
  const child = spawn('node', [serverPath], {
    cwd: rootDir,
    stdio: 'inherit',
    env: { ...process.env }
  });
  
  child.on('error', (err) => {
    console.error('Failed to start server:', err);
    process.exit(1);
  });
  
  child.on('exit', (code) => {
    process.exit(code || 0);
  });
}

async function startDevelopment() {
  console.log('Starting Moatless in development mode...');
  
  const child = spawn('pnpm', ['dev'], {
    cwd: rootDir,
    stdio: 'inherit',
    shell: true
  });
  
  child.on('error', (err) => {
    console.error('Failed to start development server:', err);
    process.exit(1);
  });
  
  child.on('exit', (code) => {
    process.exit(code || 0);
  });
}

async function checkHealth() {
  const port = process.env.PORT || 8000;
  const url = `http://localhost:${port}/health`;
  
  try {
    const response = await fetch(url);
    const health = await response.json();
    
    console.log('Health Status:', health.status);
    console.log('Uptime:', `${Math.round(health.uptime)}s`);
    console.log('Version:', health.version);
    
    if (health.status !== 'healthy') {
      console.log('\nHealth Checks:');
      for (const check of health.checks) {
        const status = check.status === 'healthy' ? '✓' : '✗';
        console.log(`  ${status} ${check.name}: ${check.message}`);
      }
    }
    
    process.exit(health.status === 'healthy' ? 0 : 1);
  } catch (error) {
    console.error('Health check failed:', error.message);
    process.exit(1);
  }
}

async function showVersion() {
  try {
    const packagePath = join(rootDir, 'package.json');
    const packageJson = JSON.parse(await fs.readFile(packagePath, 'utf-8'));
    console.log(packageJson.version);
  } catch (error) {
    console.log('unknown');
  }
}

function showHelp() {
  console.log(`
Moatless - AI-powered code analysis and task management

Usage:
  npx moatless [command]

Commands:
  start     Start the Moatless server (default)
  health    Check server health status
  version   Show version information
  help      Show this help message

Environment Variables:
  PORT      Server port (default: 8000)
  HOST      Server host (default: 0.0.0.0)
  DATA_DIR  Data storage directory (default: ./data)
  LOG_LEVEL Logging level (default: info)

Examples:
  npx moatless                    # Start server
  PORT=3000 npx moatless start    # Start on port 3000
  npx moatless health             # Check health
  npx moatless --help             # Show help
`);
}
```

## Testing Strategy

### End-to-End Tests

#### e2e/integration.test.ts
```typescript
import { test, expect } from '@playwright/test';

test.describe('Moatless Integration', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
  });
  
  test('should load the application', async ({ page }) => {
    // Wait for the app to load
    await expect(page.locator('h1')).toBeVisible();
    
    // Check that the page title is correct
    await expect(page).toHaveTitle(/Moatless/);
  });
  
  test('should create and manage tasks', async ({ page }) => {
    // Navigate to task creation
    await page.click('[data-testid="create-task-button"]');
    
    // Fill out task form
    await page.fill('[data-testid="task-message"]', 'Test task message');
    await page.selectOption('[data-testid="environment-select"]', '1');
    
    // Submit form
    await page.click('[data-testid="submit-task"]');
    
    // Verify task was created
    await expect(page.locator('[data-testid="task-list"]')).toContainText('Test task message');
  });
  
  test('should display real-time events', async ({ page }) => {
    // Create a task first
    await page.click('[data-testid="create-task-button"]');
    await page.fill('[data-testid="task-message"]', 'Real-time test');
    await page.selectOption('[data-testid="environment-select"]', '1');
    await page.click('[data-testid="submit-task"]');
    
    // Go to task detail
    await page.click('[data-testid="task-item"]:first-child');
    
    // Verify WebSocket connection
    await expect(page.locator('[data-testid="connection-status"]')).toContainText('Connected');
    
    // Send a message and verify it appears
    await page.fill('[data-testid="message-input"]', 'Hello from test');
    await page.click('[data-testid="send-message"]');
    
    // Check for events
    await expect(page.locator('[data-testid="event-list"]')).toContainText('Hello from test');
  });
  
  test('should handle API errors gracefully', async ({ page }) => {
    // Mock a network error
    await page.route('/api/tasks', route => {
      route.fulfill({
        status: 500,
        contentType: 'application/json',
        body: JSON.stringify({
          error: {
            code: 'INTERNAL_SERVER_ERROR',
            message: 'Test error'
          }
        })
      });
    });
    
    // Try to load tasks
    await page.reload();
    
    // Verify error handling
    await expect(page.locator('[data-testid="error-message"]')).toContainText('Test error');
  });
});
```

## Migration Checklist

- [ ] Update client build configuration for production
- [ ] Integrate API client with typed endpoints
- [ ] Set up WebSocket connection for real-time events
- [ ] Configure static file serving in production
- [ ] Update Docker configuration for deployment
- [ ] Set up NPX distribution package
- [ ] Add environment-specific configuration
- [ ] Implement client-server error handling
- [ ] Create comprehensive end-to-end tests
- [ ] Set up production monitoring and logging
- [ ] Configure security headers and CORS
- [ ] Add health check endpoints
- [ ] Test production build and deployment
- [ ] Validate NPX installation and usage

## Production Deployment

### 1. Build Process
```bash
# Build for production
NODE_ENV=production pnpm build:production

# Test the build locally
node dist/server/index.js

# Package for NPX distribution
pnpm pack
```

### 2. Environment Setup
```bash
# Production environment variables
export NODE_ENV=production
export PORT=8000
export LOG_LEVEL=info
export STORAGE_PATH=/app/data
export PERFORMANCE_MONITORING=true
```

### 3. Health Monitoring
```bash
# Check health
curl http://localhost:8000/health

# Monitor logs
tail -f logs/moatless.log

# Check metrics
curl http://localhost:8000/health/memory
```

## Performance Optimization

### 1. Client Optimization
- Code splitting and lazy loading
- Bundle size optimization
- Asset compression and caching
- Service worker implementation

### 2. Server Optimization
- Response compression
- Static file caching
- Database query optimization
- Memory usage monitoring

### 3. Network Optimization
- CDN integration
- HTTP/2 support
- WebSocket connection pooling
- Request/response optimization

## Security Considerations

### 1. Client Security
- Content Security Policy
- XSS prevention
- CSRF protection
- Secure cookie handling

### 2. Server Security
- Input validation and sanitization
- Rate limiting
- Authentication and authorization
- Security headers

### 3. Deployment Security
- Environment variable protection
- Container security
- Network security
- SSL/TLS configuration

## Next Steps

1. Complete all migration phases
2. Run comprehensive testing
3. Set up CI/CD pipeline
4. Deploy to production environment
5. Monitor performance and reliability
6. Gather user feedback and iterate