# Service Integration Migration Guide

This guide covers the integration of all services into a cohesive system with dependency injection, lifecycle management, and cross-service communication.

## Overview

Phase 9 focuses on creating the service integration layer that orchestrates all the previously created services, manages their dependencies, and provides a unified interface for the application.

## Implementation

### 1. Service Registry and Dependency Injection

#### server/services/index.ts
```typescript
import type { FastifyInstance } from 'fastify';
import { createStorageService, type StorageBackend } from './storage.js';
import { createEventService, type EventService } from './event.js';
import { createEnvironmentService, type EnvironmentService } from './environment.js';
import { createWorkspaceService, type WorkspaceService } from './workspace.js';
import { createTaskService, type TaskService } from './task.js';
import { createAgentService, type AgentService } from './agent.js';
import { logger } from '../utils/logger.js';
import { performanceMonitor } from '../utils/performance.js';

export interface Services {
  storage: StorageBackend;
  eventService: EventService;
  environmentService: EnvironmentService;
  workspaceService: WorkspaceService;
  taskService: TaskService;
  agentService: AgentService;
}

export interface ServiceConfig {
  storage: {
    type: 'file' | 'memory' | 'redis';
    path?: string;
    connectionString?: string;
  };
  events: {
    maxListeners: number;
    batchSize: number;
  };
  performance: {
    monitoring: boolean;
    metricsInterval: number;
  };
  cleanup: {
    interval: number;
    maxAge: number;
  };
}

export class ServiceContainer {
  private services: Partial<Services> = {};
  private initialized = false;
  private cleanupInterval?: NodeJS.Timeout;
  
  constructor(
    private app: FastifyInstance,
    private config: ServiceConfig
  ) {}
  
  async initialize(): Promise<Services> {
    if (this.initialized) {
      return this.services as Services;
    }
    
    logger.info('Initializing services...');
    const startTime = performance.now();
    
    try {
      // Initialize services in dependency order
      await this.initializeStorage();
      await this.initializeEventService();
      await this.initializeEnvironmentService();
      await this.initializeWorkspaceService();
      await this.initializeTaskService();
      await this.initializeAgentService();
      
      // Set up cross-service integrations
      await this.setupServiceIntegrations();
      
      // Start background processes
      this.startBackgroundProcesses();
      
      this.initialized = true;
      
      const duration = performance.now() - startTime;
      logger.info('Services initialized successfully', { duration: `${duration}ms` });
      
      return this.services as Services;
      
    } catch (error) {
      logger.error('Failed to initialize services', error as Error);
      await this.cleanup();
      throw error;
    }
  }
  
  async cleanup(): Promise<void> {
    logger.info('Cleaning up services...');
    
    try {
      // Stop background processes
      if (this.cleanupInterval) {
        clearInterval(this.cleanupInterval);
      }
      
      // Cleanup services in reverse order
      if (this.services.agentService) {
        await this.cleanupAgentService();
      }
      
      if (this.services.taskService) {
        await this.cleanupTaskService();
      }
      
      if (this.services.workspaceService) {
        await this.cleanupWorkspaceService();
      }
      
      if (this.services.eventService) {
        await this.cleanupEventService();
      }
      
      // Storage cleanup last
      if (this.services.storage) {
        await this.cleanupStorage();
      }
      
      this.services = {};
      this.initialized = false;
      
      logger.info('Services cleanup completed');
      
    } catch (error) {
      logger.error('Error during service cleanup', error as Error);
    }
  }
  
  getService<K extends keyof Services>(name: K): Services[K] {
    if (!this.initialized) {
      throw new Error('Services not initialized. Call initialize() first.');
    }
    
    const service = this.services[name];
    if (!service) {
      throw new Error(`Service '${name}' not found`);
    }
    
    return service;
  }
  
  getAllServices(): Services {
    if (!this.initialized) {
      throw new Error('Services not initialized. Call initialize() first.');
    }
    
    return this.services as Services;
  }
  
  private async initializeStorage(): Promise<void> {
    logger.debug('Initializing storage service...');
    
    this.services.storage = createStorageService(this.app);
    
    // Test storage connectivity
    await this.services.storage.save('_health_check', { timestamp: new Date() });
    const healthCheck = await this.services.storage.get('_health_check');
    
    if (!healthCheck) {
      throw new Error('Storage health check failed');
    }
    
    await this.services.storage.delete('_health_check');
    logger.debug('Storage service initialized');
  }
  
  private async initializeEventService(): Promise<void> {
    logger.debug('Initializing event service...');
    
    if (!this.services.storage) {
      throw new Error('Storage service must be initialized before event service');
    }
    
    this.services.eventService = createEventService(this.app, this.services.storage);
    this.services.eventService.setMaxListeners(this.config.events.maxListeners);
    
    // Set up event monitoring
    this.services.eventService.on('error', (error) => {
      logger.error('Event service error', error);
    });
    
    logger.debug('Event service initialized');
  }
  
  private async initializeEnvironmentService(): Promise<void> {
    logger.debug('Initializing environment service...');
    
    if (!this.services.storage) {
      throw new Error('Storage service must be initialized before environment service');
    }
    
    this.services.environmentService = createEnvironmentService(this.app, this.services.storage);
    
    logger.debug('Environment service initialized');
  }
  
  private async initializeWorkspaceService(): Promise<void> {
    logger.debug('Initializing workspace service...');
    
    if (!this.services.storage || !this.services.environmentService || !this.services.eventService) {
      throw new Error('Required services must be initialized before workspace service');
    }
    
    this.services.workspaceService = createWorkspaceService(
      this.app,
      this.services.storage,
      this.services.environmentService,
      this.services.eventService
    );
    
    logger.debug('Workspace service initialized');
  }
  
  private async initializeTaskService(): Promise<void> {
    logger.debug('Initializing task service...');
    
    if (!this.services.storage || !this.services.eventService || 
        !this.services.environmentService || !this.services.workspaceService) {
      throw new Error('Required services must be initialized before task service');
    }
    
    this.services.taskService = createTaskService(
      this.app,
      this.services.storage,
      this.services.eventService,
      this.services.environmentService,
      this.services.workspaceService
    );
    
    logger.debug('Task service initialized');
  }
  
  private async initializeAgentService(): Promise<void> {
    logger.debug('Initializing agent service...');
    
    if (!this.services.taskService || !this.services.workspaceService || !this.services.eventService) {
      throw new Error('Required services must be initialized before agent service');
    }
    
    this.services.agentService = createAgentService(
      this.app,
      this.services.taskService,
      this.services.workspaceService,
      this.services.eventService
    );
    
    logger.debug('Agent service initialized');
  }
  
  private async setupServiceIntegrations(): Promise<void> {
    logger.debug('Setting up service integrations...');
    
    // Set up event handlers for cross-service communication
    this.setupTaskEventHandlers();
    this.setupWorkspaceEventHandlers();
    this.setupAgentEventHandlers();
    
    // Set up performance monitoring
    if (this.config.performance.monitoring) {
      this.setupPerformanceMonitoring();
    }
    
    logger.debug('Service integrations configured');
  }
  
  private setupTaskEventHandlers(): void {
    const { eventService, workspaceService } = this.services;
    
    if (!eventService || !workspaceService) return;
    
    // Auto-cleanup workspaces when tasks are deleted
    eventService.subscribeByType('stop', async (event) => {
      if (event.subType === 'task_deleted') {
        try {
          await workspaceService.cleanupWorkspace(event.taskId);
        } catch (error) {
          logger.warn('Failed to cleanup workspace on task deletion', error as Error, {
            taskId: event.taskId
          });
        }
      }
    });
  }
  
  private setupWorkspaceEventHandlers(): void {
    const { eventService, taskService } = this.services;
    
    if (!eventService || !taskService) return;
    
    // Update task status when workspace setup fails
    eventService.subscribeByType('workspace', async (event) => {
      if (event.subType === 'setup_failed') {
        try {
          await taskService.updateTask(event.taskId, { status: 'error' });
        } catch (error) {
          logger.warn('Failed to update task status on workspace failure', error as Error, {
            taskId: event.taskId
          });
        }
      }
    });
  }
  
  private setupAgentEventHandlers(): void {
    const { eventService, taskService } = this.services;
    
    if (!eventService || !taskService) return;
    
    // Update task on agent completion
    eventService.subscribeByType('completion', async (event) => {
      try {
        await taskService.updateTask(event.taskId, { status: 'completed' });
      } catch (error) {
        logger.warn('Failed to update task status on completion', error as Error, {
          taskId: event.taskId
        });
      }
    });
  }
  
  private setupPerformanceMonitoring(): void {
    // Monitor service health metrics
    setInterval(() => {
      const memoryUsage = process.memoryUsage();
      performanceMonitor.recordMetric('memory_heap_used', memoryUsage.heapUsed, 'bytes');
      performanceMonitor.recordMetric('memory_heap_total', memoryUsage.heapTotal, 'bytes');
      performanceMonitor.recordMetric('memory_rss', memoryUsage.rss, 'bytes');
      
      // Monitor event service health
      if (this.services.eventService) {
        const listenerCount = this.services.eventService.listenerCount('event');
        performanceMonitor.recordMetric('event_listeners', listenerCount, 'count');
      }
      
    }, this.config.performance.metricsInterval);
  }
  
  private startBackgroundProcesses(): void {
    // Periodic cleanup of expired data
    this.cleanupInterval = setInterval(async () => {
      try {
        await this.performPeriodicCleanup();
      } catch (error) {
        logger.error('Periodic cleanup failed', error as Error);
      }
    }, this.config.cleanup.interval);
  }
  
  private async performPeriodicCleanup(): Promise<void> {
    logger.debug('Performing periodic cleanup...');
    
    const cutoffDate = new Date(Date.now() - this.config.cleanup.maxAge);
    
    // Clean up old events
    if (this.services.storage) {
      try {
        const eventKeys = await this.services.storage.list('events/');
        for (const key of eventKeys) {
          const events = await this.services.storage.get(key);
          if (events && Array.isArray(events)) {
            const filteredEvents = events.filter((event: any) => 
              new Date(event.timestamp) > cutoffDate
            );
            
            if (filteredEvents.length !== events.length) {
              await this.services.storage.save(key, filteredEvents);
            }
          }
        }
      } catch (error) {
        logger.warn('Failed to cleanup old events', error as Error);
      }
    }
    
    // Clean up orphaned workspaces
    if (this.services.workspaceService && this.services.taskService) {
      try {
        const workspaces = await this.services.workspaceService.listWorkspaces();
        for (const workspace of workspaces) {
          const task = await this.services.taskService.getTask(workspace.id);
          if (!task) {
            logger.info('Cleaning up orphaned workspace', { workspaceId: workspace.id });
            await this.services.workspaceService.cleanupWorkspace(workspace.id);
          }
        }
      } catch (error) {
        logger.warn('Failed to cleanup orphaned workspaces', error as Error);
      }
    }
  }
  
  private async cleanupStorage(): Promise<void> {
    // Storage cleanup is usually handled by the implementation
    logger.debug('Storage cleanup completed');
  }
  
  private async cleanupEventService(): Promise<void> {
    if (this.services.eventService) {
      this.services.eventService.removeAllListeners();
    }
    logger.debug('Event service cleanup completed');
  }
  
  private async cleanupWorkspaceService(): Promise<void> {
    // Cleanup active workspaces
    if (this.services.workspaceService) {
      const workspaces = await this.services.workspaceService.listWorkspaces();
      for (const workspace of workspaces) {
        if (workspace.status === 'ready') {
          await this.services.workspaceService.cleanupWorkspace(workspace.id);
        }
      }
    }
    logger.debug('Workspace service cleanup completed');
  }
  
  private async cleanupTaskService(): Promise<void> {
    // Wait for background tasks to complete
    if (this.services.taskService) {
      await this.services.taskService.waitForBackgroundTasks();
    }
    logger.debug('Task service cleanup completed');
  }
  
  private async cleanupAgentService(): Promise<void> {
    // Stop all running agents
    if (this.services.agentService) {
      const activeAgents = await this.services.agentService.listActiveAgents();
      for (const execution of activeAgents) {
        await this.services.agentService.stopAgent(execution.id);
      }
    }
    logger.debug('Agent service cleanup completed');
  }
}

export function setupServices(app: FastifyInstance, config?: Partial<ServiceConfig>): ServiceContainer {
  const defaultConfig: ServiceConfig = {
    storage: {
      type: 'file',
      path: process.env.DATA_DIR || './data'
    },
    events: {
      maxListeners: 100,
      batchSize: 10
    },
    performance: {
      monitoring: process.env.NODE_ENV === 'production',
      metricsInterval: 30000 // 30 seconds
    },
    cleanup: {
      interval: 300000, // 5 minutes
      maxAge: 86400000 // 24 hours
    }
  };
  
  const finalConfig = { ...defaultConfig, ...config };
  
  return new ServiceContainer(app, finalConfig);
}

// Extend Fastify instance type
declare module 'fastify' {
  interface FastifyInstance {
    services: Services;
    serviceContainer: ServiceContainer;
  }
}
```

### 2. Service Health Monitoring

#### server/services/health.ts
```typescript
import type { FastifyInstance } from 'fastify';
import type { Services } from './index.js';
import { logger } from '../utils/logger.js';

export interface HealthCheck {
  name: string;
  status: 'healthy' | 'unhealthy' | 'degraded';
  message?: string;
  timestamp: Date;
  responseTime?: number;
  details?: any;
}

export interface SystemHealth {
  status: 'healthy' | 'unhealthy' | 'degraded';
  checks: HealthCheck[];
  timestamp: Date;
  uptime: number;
  version: string;
}

export class HealthMonitor {
  private checks: Map<string, () => Promise<HealthCheck>> = new Map();
  
  constructor(private services: Services) {
    this.registerDefaultChecks();
  }
  
  registerCheck(name: string, check: () => Promise<HealthCheck>): void {
    this.checks.set(name, check);
  }
  
  async runAllChecks(): Promise<SystemHealth> {
    const startTime = Date.now();
    const checks: HealthCheck[] = [];
    
    for (const [name, check] of this.checks) {
      try {
        const result = await check();
        checks.push(result);
      } catch (error) {
        checks.push({
          name,
          status: 'unhealthy',
          message: error instanceof Error ? error.message : String(error),
          timestamp: new Date()
        });
      }
    }
    
    const overallStatus = this.calculateOverallStatus(checks);
    
    return {
      status: overallStatus,
      checks,
      timestamp: new Date(),
      uptime: process.uptime(),
      version: process.env.npm_package_version || '1.0.0'
    };
  }
  
  async runCheck(name: string): Promise<HealthCheck> {
    const check = this.checks.get(name);
    if (!check) {
      throw new Error(`Health check '${name}' not found`);
    }
    
    return await check();
  }
  
  private registerDefaultChecks(): void {
    // Storage health check
    this.registerCheck('storage', async () => {
      const startTime = Date.now();
      
      try {
        const testKey = `health_check_${Date.now()}`;
        await this.services.storage.save(testKey, { timestamp: new Date() });
        const result = await this.services.storage.get(testKey);
        await this.services.storage.delete(testKey);
        
        if (!result) {
          throw new Error('Storage read/write test failed');
        }
        
        return {
          name: 'storage',
          status: 'healthy',
          message: 'Storage is accessible',
          timestamp: new Date(),
          responseTime: Date.now() - startTime
        };
      } catch (error) {
        return {
          name: 'storage',
          status: 'unhealthy',
          message: error instanceof Error ? error.message : String(error),
          timestamp: new Date(),
          responseTime: Date.now() - startTime
        };
      }
    });
    
    // Event service health check
    this.registerCheck('events', async () => {
      const startTime = Date.now();
      
      try {
        const listenerCount = this.services.eventService.listenerCount('event');
        
        return {
          name: 'events',
          status: 'healthy',
          message: 'Event service is operational',
          timestamp: new Date(),
          responseTime: Date.now() - startTime,
          details: {
            listeners: listenerCount
          }
        };
      } catch (error) {
        return {
          name: 'events',
          status: 'unhealthy',
          message: error instanceof Error ? error.message : String(error),
          timestamp: new Date(),
          responseTime: Date.now() - startTime
        };
      }
    });
    
    // Memory health check
    this.registerCheck('memory', async () => {
      const startTime = Date.now();
      const memUsage = process.memoryUsage();
      const heapUsedMB = memUsage.heapUsed / 1024 / 1024;
      const heapTotalMB = memUsage.heapTotal / 1024 / 1024;
      const heapUtilization = (heapUsedMB / heapTotalMB) * 100;
      
      let status: 'healthy' | 'degraded' | 'unhealthy' = 'healthy';
      let message = 'Memory usage is normal';
      
      if (heapUtilization > 90) {
        status = 'unhealthy';
        message = 'Memory usage is critically high';
      } else if (heapUtilization > 75) {
        status = 'degraded';
        message = 'Memory usage is elevated';
      }
      
      return {
        name: 'memory',
        status,
        message,
        timestamp: new Date(),
        responseTime: Date.now() - startTime,
        details: {
          heapUsedMB: Math.round(heapUsedMB),
          heapTotalMB: Math.round(heapTotalMB),
          heapUtilization: Math.round(heapUtilization),
          rssMB: Math.round(memUsage.rss / 1024 / 1024)
        }
      };
    });
    
    // Environment service health check
    this.registerCheck('environments', async () => {
      const startTime = Date.now();
      
      try {
        const environments = await this.services.environmentService.listEnvironments();
        
        return {
          name: 'environments',
          status: 'healthy',
          message: 'Environment service is operational',
          timestamp: new Date(),
          responseTime: Date.now() - startTime,
          details: {
            count: environments.length
          }
        };
      } catch (error) {
        return {
          name: 'environments',
          status: 'unhealthy',
          message: error instanceof Error ? error.message : String(error),
          timestamp: new Date(),
          responseTime: Date.now() - startTime
        };
      }
    });
    
    // Workspace service health check
    this.registerCheck('workspaces', async () => {
      const startTime = Date.now();
      
      try {
        const workspaces = await this.services.workspaceService.listWorkspaces();
        const activeWorkspaces = workspaces.filter(w => w.status === 'ready');
        
        return {
          name: 'workspaces',
          status: 'healthy',
          message: 'Workspace service is operational',
          timestamp: new Date(),
          responseTime: Date.now() - startTime,
          details: {
            total: workspaces.length,
            active: activeWorkspaces.length
          }
        };
      } catch (error) {
        return {
          name: 'workspaces',
          status: 'unhealthy',
          message: error instanceof Error ? error.message : String(error),
          timestamp: new Date(),
          responseTime: Date.now() - startTime
        };
      }
    });
    
    // Agent service health check
    this.registerCheck('agents', async () => {
      const startTime = Date.now();
      
      try {
        const activeAgents = await this.services.agentService.listActiveAgents();
        
        return {
          name: 'agents',
          status: 'healthy',
          message: 'Agent service is operational',
          timestamp: new Date(),
          responseTime: Date.now() - startTime,
          details: {
            active: activeAgents.length
          }
        };
      } catch (error) {
        return {
          name: 'agents',
          status: 'unhealthy',
          message: error instanceof Error ? error.message : String(error),
          timestamp: new Date(),
          responseTime: Date.now() - startTime
        };
      }
    });
  }
  
  private calculateOverallStatus(checks: HealthCheck[]): 'healthy' | 'unhealthy' | 'degraded' {
    const hasUnhealthy = checks.some(check => check.status === 'unhealthy');
    const hasDegraded = checks.some(check => check.status === 'degraded');
    
    if (hasUnhealthy) {
      return 'unhealthy';
    } else if (hasDegraded) {
      return 'degraded';
    } else {
      return 'healthy';
    }
  }
}

export function createHealthMonitor(services: Services): HealthMonitor {
  return new HealthMonitor(services);
}
```

### 3. Service Configuration Management

#### server/config/service-config.ts
```typescript
import { z } from 'zod';

const ServiceConfigSchema = z.object({
  storage: z.object({
    type: z.enum(['file', 'memory', 'redis']).default('file'),
    path: z.string().optional(),
    connectionString: z.string().optional(),
    maxSize: z.number().positive().optional(),
    ttl: z.number().positive().optional()
  }),
  
  events: z.object({
    maxListeners: z.number().positive().default(100),
    batchSize: z.number().positive().default(10),
    retentionDays: z.number().positive().default(7)
  }),
  
  tasks: z.object({
    maxConcurrent: z.number().positive().default(10),
    timeoutMs: z.number().positive().default(300000), // 5 minutes
    cleanupIntervalMs: z.number().positive().default(3600000) // 1 hour
  }),
  
  workspaces: z.object({
    baseDir: z.string().default('./workspaces'),
    maxAge: z.number().positive().default(86400000), // 24 hours
    cleanupOrphaned: z.boolean().default(true)
  }),
  
  agents: z.object({
    maxConcurrent: z.number().positive().default(5),
    timeoutMs: z.number().positive().default(600000), // 10 minutes
    retryAttempts: z.number().nonnegative().default(3)
  }),
  
  performance: z.object({
    monitoring: z.boolean().default(false),
    metricsInterval: z.number().positive().default(30000), // 30 seconds
    memoryThreshold: z.number().min(0).max(1).default(0.8) // 80%
  }),
  
  health: z.object({
    checkInterval: z.number().positive().default(60000), // 1 minute
    endpoint: z.string().default('/health'),
    enabled: z.boolean().default(true)
  }),
  
  logging: z.object({
    level: z.enum(['trace', 'debug', 'info', 'warn', 'error', 'fatal']).default('info'),
    file: z.string().optional(),
    structured: z.boolean().default(true)
  })
});

export type ServiceConfig = z.infer<typeof ServiceConfigSchema>;

export function loadServiceConfig(): ServiceConfig {
  const config = {
    storage: {
      type: process.env.STORAGE_TYPE || 'file',
      path: process.env.STORAGE_PATH,
      connectionString: process.env.STORAGE_CONNECTION_STRING,
      maxSize: process.env.STORAGE_MAX_SIZE ? parseInt(process.env.STORAGE_MAX_SIZE) : undefined,
      ttl: process.env.STORAGE_TTL ? parseInt(process.env.STORAGE_TTL) : undefined
    },
    
    events: {
      maxListeners: process.env.EVENTS_MAX_LISTENERS ? parseInt(process.env.EVENTS_MAX_LISTENERS) : undefined,
      batchSize: process.env.EVENTS_BATCH_SIZE ? parseInt(process.env.EVENTS_BATCH_SIZE) : undefined,
      retentionDays: process.env.EVENTS_RETENTION_DAYS ? parseInt(process.env.EVENTS_RETENTION_DAYS) : undefined
    },
    
    tasks: {
      maxConcurrent: process.env.TASKS_MAX_CONCURRENT ? parseInt(process.env.TASKS_MAX_CONCURRENT) : undefined,
      timeoutMs: process.env.TASKS_TIMEOUT_MS ? parseInt(process.env.TASKS_TIMEOUT_MS) : undefined,
      cleanupIntervalMs: process.env.TASKS_CLEANUP_INTERVAL_MS ? parseInt(process.env.TASKS_CLEANUP_INTERVAL_MS) : undefined
    },
    
    workspaces: {
      baseDir: process.env.WORKSPACES_BASE_DIR,
      maxAge: process.env.WORKSPACES_MAX_AGE ? parseInt(process.env.WORKSPACES_MAX_AGE) : undefined,
      cleanupOrphaned: process.env.WORKSPACES_CLEANUP_ORPHANED === 'true'
    },
    
    agents: {
      maxConcurrent: process.env.AGENTS_MAX_CONCURRENT ? parseInt(process.env.AGENTS_MAX_CONCURRENT) : undefined,
      timeoutMs: process.env.AGENTS_TIMEOUT_MS ? parseInt(process.env.AGENTS_TIMEOUT_MS) : undefined,
      retryAttempts: process.env.AGENTS_RETRY_ATTEMPTS ? parseInt(process.env.AGENTS_RETRY_ATTEMPTS) : undefined
    },
    
    performance: {
      monitoring: process.env.PERFORMANCE_MONITORING === 'true',
      metricsInterval: process.env.PERFORMANCE_METRICS_INTERVAL ? parseInt(process.env.PERFORMANCE_METRICS_INTERVAL) : undefined,
      memoryThreshold: process.env.PERFORMANCE_MEMORY_THRESHOLD ? parseFloat(process.env.PERFORMANCE_MEMORY_THRESHOLD) : undefined
    },
    
    health: {
      checkInterval: process.env.HEALTH_CHECK_INTERVAL ? parseInt(process.env.HEALTH_CHECK_INTERVAL) : undefined,
      endpoint: process.env.HEALTH_ENDPOINT,
      enabled: process.env.HEALTH_ENABLED !== 'false'
    },
    
    logging: {
      level: process.env.LOG_LEVEL,
      file: process.env.LOG_FILE,
      structured: process.env.LOG_STRUCTURED !== 'false'
    }
  };
  
  // Remove undefined values
  const cleanConfig = JSON.parse(JSON.stringify(config, (key, value) => 
    value === undefined ? undefined : value
  ));
  
  const result = ServiceConfigSchema.safeParse(cleanConfig);
  
  if (!result.success) {
    throw new Error(`Invalid service configuration: ${result.error.message}`);
  }
  
  return result.data;
}

export function validateServiceConfig(config: any): ServiceConfig {
  const result = ServiceConfigSchema.safeParse(config);
  
  if (!result.success) {
    throw new Error(`Invalid service configuration: ${result.error.message}`);
  }
  
  return result.data;
}
```

### 4. Service Lifecycle Hooks

#### server/plugins/service-lifecycle.ts
```typescript
import type { FastifyInstance } from 'fastify';
import fastifyPlugin from 'fastify-plugin';
import { setupServices, type ServiceContainer } from '../services/index.js';
import { createHealthMonitor, type HealthMonitor } from '../services/health.js';
import { loadServiceConfig } from '../config/service-config.js';
import { logger } from '../utils/logger.js';

declare module 'fastify' {
  interface FastifyInstance {
    serviceContainer: ServiceContainer;
    healthMonitor: HealthMonitor;
  }
}

async function serviceLifecyclePlugin(app: FastifyInstance) {
  const config = loadServiceConfig();
  
  // Create service container
  const serviceContainer = setupServices(app, config);
  
  // Initialize services
  const services = await serviceContainer.initialize();
  
  // Decorate Fastify instance with services
  app.decorate('services', services);
  app.decorate('serviceContainer', serviceContainer);
  
  // Create and register health monitor
  const healthMonitor = createHealthMonitor(services);
  app.decorate('healthMonitor', healthMonitor);
  
  // Add health check route if enabled
  if (config.health.enabled) {
    app.get(config.health.endpoint, async (request, reply) => {
      const health = await healthMonitor.runAllChecks();
      
      const statusCode = health.status === 'healthy' ? 200 : 
                        health.status === 'degraded' ? 200 : 503;
      
      return reply.status(statusCode).send(health);
    });
    
    // Individual health check endpoints
    app.get(`${config.health.endpoint}/:check`, async (request, reply) => {
      const { check } = request.params as { check: string };
      
      try {
        const result = await healthMonitor.runCheck(check);
        const statusCode = result.status === 'healthy' ? 200 : 503;
        
        return reply.status(statusCode).send(result);
      } catch (error) {
        return reply.status(404).send({
          error: `Health check '${check}' not found`
        });
      }
    });
  }
  
  // Set up graceful shutdown
  const cleanup = async () => {
    logger.info('Received shutdown signal, cleaning up...');
    
    try {
      await serviceContainer.cleanup();
      logger.info('Cleanup completed successfully');
    } catch (error) {
      logger.error('Error during cleanup', error as Error);
    }
  };
  
  // Register shutdown handlers
  app.addHook('onClose', cleanup);
  
  // Handle process signals
  process.on('SIGTERM', async () => {
    logger.info('Received SIGTERM, shutting down gracefully...');
    await app.close();
    process.exit(0);
  });
  
  process.on('SIGINT', async () => {
    logger.info('Received SIGINT, shutting down gracefully...');
    await app.close();
    process.exit(0);
  });
  
  // Handle uncaught exceptions
  process.on('uncaughtException', (error) => {
    logger.fatal('Uncaught exception', error);
    process.exit(1);
  });
  
  process.on('unhandledRejection', (reason, promise) => {
    logger.fatal('Unhandled rejection', reason as Error, { promise });
    process.exit(1);
  });
  
  logger.info('Service lifecycle plugin initialized', {
    healthEndpoint: config.health.enabled ? config.health.endpoint : null,
    monitoring: config.performance.monitoring
  });
}

export default fastifyPlugin(serviceLifecyclePlugin, {
  name: 'service-lifecycle',
  dependencies: []
});
```

## Testing Strategy

### Integration Tests

#### server/services/__tests__/integration.test.ts
```typescript
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { build } from '../../../test/helper.js';
import type { FastifyInstance } from 'fastify';

describe('Service Integration', () => {
  let app: FastifyInstance;
  
  beforeEach(async () => {
    app = await build();
  });
  
  afterEach(async () => {
    await app.close();
  });
  
  describe('Service Container', () => {
    it('should initialize all services', async () => {
      const services = app.services;
      
      expect(services.storage).toBeDefined();
      expect(services.eventService).toBeDefined();
      expect(services.environmentService).toBeDefined();
      expect(services.workspaceService).toBeDefined();
      expect(services.taskService).toBeDefined();
      expect(services.agentService).toBeDefined();
    });
    
    it('should handle cross-service communication', async () => {
      const { taskService, environmentService, workspaceService } = app.services;
      
      // Create environment
      const environment = await environmentService.createEnvironment({
        name: 'Test Environment',
        repository: {
          type: 'local',
          path: process.cwd()
        },
        codeExecution: {
          type: 'local'
        }
      });
      
      // Create task
      const taskId = await taskService.createTask({
        agentType: 'claude-code',
        environmentId: environment.id,
        message: 'Test task',
        start: false
      });
      
      // Workspace should be created automatically
      const workspace = await workspaceService.getWorkspace(taskId);
      expect(workspace).toBeDefined();
      expect(workspace?.environmentId).toBe(environment.id);
    });
  });
  
  describe('Health Monitoring', () => {
    it('should provide health status', async () => {
      const response = await app.inject({
        method: 'GET',
        url: '/health'
      });
      
      expect(response.statusCode).toBe(200);
      
      const health = JSON.parse(response.body);
      expect(health.status).toMatch(/healthy|degraded/);
      expect(health.checks).toBeDefined();
      expect(Array.isArray(health.checks)).toBe(true);
    });
    
    it('should provide individual health checks', async () => {
      const response = await app.inject({
        method: 'GET',
        url: '/health/storage'
      });
      
      expect(response.statusCode).toBe(200);
      
      const check = JSON.parse(response.body);
      expect(check.name).toBe('storage');
      expect(check.status).toMatch(/healthy|degraded|unhealthy/);
    });
  });
});
```

## Migration Checklist

- [ ] Create service container with dependency injection
- [ ] Implement service lifecycle management
- [ ] Add health monitoring and checks
- [ ] Create service configuration management
- [ ] Implement graceful shutdown handling
- [ ] Add cross-service event communication
- [ ] Create performance monitoring integration
- [ ] Implement periodic cleanup processes
- [ ] Add service registry and discovery
- [ ] Write comprehensive integration tests
- [ ] Test service startup and shutdown
- [ ] Validate health monitoring accuracy
- [ ] Test error scenarios and recovery
- [ ] Add monitoring and alerting

## Best Practices

### 1. Dependency Management
- Initialize services in correct dependency order
- Validate dependencies at startup
- Handle circular dependencies gracefully
- Use dependency injection for testability

### 2. Error Handling
- Implement circuit breaker patterns
- Handle service failures gracefully
- Provide meaningful error messages
- Log errors with proper context

### 3. Monitoring
- Monitor service health continuously
- Track performance metrics
- Set up alerting for critical issues
- Provide debugging information

### 4. Configuration
- Use environment-specific configurations
- Validate configuration at startup
- Support runtime configuration updates
- Document all configuration options

## Next Steps

1. Review [Client Integration](./10-client-integration.md)
2. Implement advanced monitoring and alerting
3. Add service discovery and load balancing
4. Set up automated testing pipeline
5. Configure production deployment strategies