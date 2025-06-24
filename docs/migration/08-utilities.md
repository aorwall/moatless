# Utilities & Helpers Migration Guide

This guide covers the migration of utility functions and helper modules from Python to TypeScript.

## Overview

Phase 8 focuses on migrating the utility functions that provide common functionality across the application, including file operations, async utilities, logging, and performance optimization helpers.

## Python Source Files to Migrate

- `/moatless-swarm/swarm/utils/git_utils.py` - Git utilities (already covered in Phase 6)
- `/moatless-swarm/swarm/utils/file_search.py` - File search utilities (already covered in Phase 5)
- `/moatless-swarm/swarm/utils/subprocess.py` - Process management utilities (already covered in Phase 7)
- `/moatless-swarm/swarm/utils/logging.py` - Logging utilities
- `/moatless-swarm/swarm/utils/async_utils.py` - Async utilities
- `/moatless-swarm/swarm/utils/rate_limiting.py` - Rate limiting utilities
- `/moatless-swarm/swarm/utils/performance.py` - Performance monitoring

## Implementation

### 1. Logging Utilities

#### server/utils/logger.ts
Port from: `swarm/utils/logging.py`

```typescript
import pino from 'pino';
import pretty from 'pino-pretty';
import { promises as fs } from 'fs';
import path from 'path';

export interface LoggerOptions {
  level?: 'trace' | 'debug' | 'info' | 'warn' | 'error' | 'fatal';
  service?: string;
  environment?: string;
  logFile?: string;
  pretty?: boolean;
  redact?: string[];
}

export interface LogContext {
  taskId?: string;
  userId?: string;
  requestId?: string;
  executionId?: string;
  [key: string]: any;
}

class Logger {
  private logger: pino.Logger;
  private context: LogContext = {};
  
  constructor(options: LoggerOptions = {}) {
    const level = options.level || process.env.LOG_LEVEL || 'info';
    const isDevelopment = process.env.NODE_ENV === 'development';
    
    const transport = options.pretty || isDevelopment ? {
      target: 'pino-pretty',
      options: {
        colorize: true,
        translateTime: 'HH:MM:ss Z',
        ignore: 'pid,hostname',
        singleLine: false
      }
    } : undefined;
    
    this.logger = pino({
      level,
      name: options.service || 'moatless',
      base: {
        environment: options.environment || process.env.NODE_ENV || 'development',
        service: options.service || 'moatless'
      },
      redact: {
        paths: options.redact || [
          'password',
          'token',
          'apiKey',
          'api_key',
          'authorization',
          'cookie',
          'session'
        ],
        censor: '[REDACTED]'
      },
      timestamp: pino.stdTimeFunctions.isoTime,
      transport
    });
    
    // Set up file logging if specified
    if (options.logFile) {
      this.setupFileLogging(options.logFile);
    }
  }
  
  child(context: LogContext): Logger {
    const childLogger = new Logger();
    childLogger.logger = this.logger.child(context);
    childLogger.context = { ...this.context, ...context };
    return childLogger;
  }
  
  setContext(context: LogContext): void {
    this.context = { ...this.context, ...context };
  }
  
  clearContext(): void {
    this.context = {};
  }
  
  trace(message: string, meta: any = {}): void {
    this.logger.trace({ ...this.context, ...meta }, message);
  }
  
  debug(message: string, meta: any = {}): void {
    this.logger.debug({ ...this.context, ...meta }, message);
  }
  
  info(message: string, meta: any = {}): void {
    this.logger.info({ ...this.context, ...meta }, message);
  }
  
  warn(message: string, meta: any = {}): void {
    this.logger.warn({ ...this.context, ...meta }, message);
  }
  
  error(message: string, error?: Error, meta: any = {}): void {
    const errorMeta = error ? {
      error: {
        message: error.message,
        stack: error.stack,
        name: error.name
      }
    } : {};
    
    this.logger.error({ ...this.context, ...errorMeta, ...meta }, message);
  }
  
  fatal(message: string, error?: Error, meta: any = {}): void {
    const errorMeta = error ? {
      error: {
        message: error.message,
        stack: error.stack,
        name: error.name
      }
    } : {};
    
    this.logger.fatal({ ...this.context, ...errorMeta, ...meta }, message);
  }
  
  // Request/response logging helpers
  logRequest(req: any, meta: any = {}): void {
    this.info('HTTP Request', {
      method: req.method,
      url: req.url,
      userAgent: req.headers?.['user-agent'],
      ip: req.ip,
      ...meta
    });
  }
  
  logResponse(req: any, res: any, responseTime: number, meta: any = {}): void {
    this.info('HTTP Response', {
      method: req.method,
      url: req.url,
      statusCode: res.statusCode,
      responseTime: `${responseTime}ms`,
      ...meta
    });
  }
  
  // Performance logging
  logPerformance(operation: string, duration: number, meta: any = {}): void {
    this.info('Performance', {
      operation,
      duration: `${duration}ms`,
      ...meta
    });
  }
  
  // Business logic logging
  logTaskEvent(taskId: string, event: string, meta: any = {}): void {
    this.info('Task Event', {
      taskId,
      event,
      ...meta
    });
  }
  
  logAgentEvent(taskId: string, agentType: string, event: string, meta: any = {}): void {
    this.info('Agent Event', {
      taskId,
      agentType,
      event,
      ...meta
    });
  }
  
  logWorkspaceEvent(taskId: string, workspaceType: string, event: string, meta: any = {}): void {
    this.info('Workspace Event', {
      taskId,
      workspaceType,
      event,
      ...meta
    });
  }
  
  private async setupFileLogging(logFile: string): Promise<void> {
    try {
      await fs.mkdir(path.dirname(logFile), { recursive: true });
      
      // Add file stream destination
      const fileStream = pino.destination({
        dest: logFile,
        sync: false,
        mkdir: true
      });
      
      // Create multi-stream logger
      const streams = [
        { stream: fileStream }
      ];
      
      if (process.env.NODE_ENV === 'development') {
        streams.push({
          stream: pretty({
            colorize: true,
            translateTime: 'HH:MM:ss Z'
          })
        });
      }
      
      this.logger = pino({
        level: this.logger.level
      }, pino.multistream(streams));
      
    } catch (error) {
      this.error('Failed to setup file logging', error as Error);
    }
  }
}

// Create default logger instance
export const logger = new Logger({
  service: 'moatless-server',
  environment: process.env.NODE_ENV,
  logFile: process.env.LOG_FILE
});

// Export logger class for custom instances
export { Logger };

// Helper function to create request-scoped loggers
export function createRequestLogger(requestId: string, taskId?: string): Logger {
  return logger.child({ requestId, taskId });
}

// Helper function to create task-scoped loggers
export function createTaskLogger(taskId: string): Logger {
  return logger.child({ taskId });
}
```

### 2. Async Utilities

#### server/utils/async-utils.ts
Port from: `swarm/utils/async_utils.py`

```typescript
export interface RetryOptions {
  maxAttempts: number;
  baseDelay: number;
  maxDelay: number;
  exponentialBackoff: boolean;
  jitter: boolean;
  retryIf?: (error: Error) => boolean;
  onRetry?: (attempt: number, error: Error) => void;
}

export class TimeoutError extends Error {
  constructor(message: string, public timeout: number) {
    super(message);
    this.name = 'TimeoutError';
  }
}

export class RetryError extends Error {
  constructor(
    message: string,
    public attempts: number,
    public lastError: Error
  ) {
    super(message);
    this.name = 'RetryError';
  }
}

/**
 * Retry a function with exponential backoff
 */
export async function retry<T>(
  fn: () => Promise<T>,
  options: Partial<RetryOptions> = {}
): Promise<T> {
  const {
    maxAttempts = 3,
    baseDelay = 1000,
    maxDelay = 10000,
    exponentialBackoff = true,
    jitter = true,
    retryIf = () => true,
    onRetry
  } = options;
  
  let lastError: Error = new Error('Unknown error');
  
  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    try {
      return await fn();
    } catch (error) {
      lastError = error as Error;
      
      if (attempt === maxAttempts || !retryIf(lastError)) {
        throw new RetryError(
          `Failed after ${attempt} attempts: ${lastError.message}`,
          attempt,
          lastError
        );
      }
      
      if (onRetry) {
        onRetry(attempt, lastError);
      }
      
      // Calculate delay
      let delay = baseDelay;
      if (exponentialBackoff) {
        delay = Math.min(baseDelay * Math.pow(2, attempt - 1), maxDelay);
      }
      
      // Add jitter
      if (jitter) {
        delay += Math.random() * delay * 0.1;
      }
      
      await sleep(delay);
    }
  }
  
  throw lastError;
}

/**
 * Add timeout to a promise
 */
export async function withTimeout<T>(
  promise: Promise<T>,
  timeoutMs: number,
  timeoutMessage?: string
): Promise<T> {
  const timeoutPromise = new Promise<never>((_, reject) => {
    setTimeout(() => {
      reject(new TimeoutError(
        timeoutMessage || `Operation timed out after ${timeoutMs}ms`,
        timeoutMs
      ));
    }, timeoutMs);
  });
  
  return Promise.race([promise, timeoutPromise]);
}

/**
 * Sleep for specified milliseconds
 */
export async function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}

/**
 * Debounce function calls
 */
export function debounce<T extends (...args: any[]) => any>(
  func: T,
  wait: number,
  immediate: boolean = false
): T {
  let timeout: NodeJS.Timeout | null = null;
  
  return ((...args: Parameters<T>) => {
    const later = () => {
      timeout = null;
      if (!immediate) func(...args);
    };
    
    const callNow = immediate && !timeout;
    
    if (timeout) clearTimeout(timeout);
    timeout = setTimeout(later, wait);
    
    if (callNow) func(...args);
  }) as T;
}

/**
 * Throttle function calls
 */
export function throttle<T extends (...args: any[]) => any>(
  func: T,
  limit: number
): T {
  let inThrottle: boolean = false;
  
  return ((...args: Parameters<T>) => {
    if (!inThrottle) {
      func(...args);
      inThrottle = true;
      setTimeout(() => inThrottle = false, limit);
    }
  }) as T;
}

/**
 * Execute promises in batches with concurrency limit
 */
export async function batchExecute<T, R>(
  items: T[],
  executor: (item: T) => Promise<R>,
  batchSize: number = 10
): Promise<R[]> {
  const results: R[] = [];
  
  for (let i = 0; i < items.length; i += batchSize) {
    const batch = items.slice(i, i + batchSize);
    const batchPromises = batch.map(executor);
    const batchResults = await Promise.all(batchPromises);
    results.push(...batchResults);
  }
  
  return results;
}

/**
 * Execute promises with concurrency limit
 */
export async function concurrent<T, R>(
  items: T[],
  executor: (item: T) => Promise<R>,
  concurrency: number = 10
): Promise<R[]> {
  const results: R[] = new Array(items.length);
  const executing: Promise<void>[] = [];
  
  for (let i = 0; i < items.length; i++) {
    const promise = executor(items[i]).then(result => {
      results[i] = result;
    });
    
    executing.push(promise);
    
    if (executing.length >= concurrency) {
      await Promise.race(executing);
      const completedIndex = executing.findIndex(p => p === promise);
      if (completedIndex !== -1) {
        executing.splice(completedIndex, 1);
      }
    }
  }
  
  await Promise.all(executing);
  return results;
}

/**
 * Create a cancelable promise
 */
export function makeCancelable<T>(promise: Promise<T>): {
  promise: Promise<T>;
  cancel: () => void;
  isCanceled: () => boolean;
} {
  let isCanceled = false;
  
  const wrappedPromise = new Promise<T>((resolve, reject) => {
    promise
      .then(value => {
        if (!isCanceled) resolve(value);
      })
      .catch(error => {
        if (!isCanceled) reject(error);
      });
  });
  
  return {
    promise: wrappedPromise,
    cancel: () => { isCanceled = true; },
    isCanceled: () => isCanceled
  };
}

/**
 * Memoize async function results
 */
export function memoizeAsync<T extends (...args: any[]) => Promise<any>>(
  fn: T,
  keyGenerator: (...args: Parameters<T>) => string = (...args) => JSON.stringify(args),
  ttl: number = 0
): T {
  const cache = new Map<string, { value: ReturnType<T>; timestamp: number }>();
  
  return ((...args: Parameters<T>): ReturnType<T> => {
    const key = keyGenerator(...args);
    const cached = cache.get(key);
    
    if (cached && (ttl === 0 || Date.now() - cached.timestamp < ttl)) {
      return cached.value;
    }
    
    const promise = fn(...args);
    cache.set(key, { value: promise, timestamp: Date.now() });
    
    // Clean up cache on error
    promise.catch(() => cache.delete(key));
    
    return promise;
  }) as T;
}
```

### 3. Rate Limiting Utilities

#### server/utils/rate-limiter.ts
Port from: `swarm/utils/rate_limiting.py`

```typescript
export interface RateLimitOptions {
  windowMs: number;
  maxRequests: number;
  keyGenerator?: (req: any) => string;
  skipSuccessfulRequests?: boolean;
  skipFailedRequests?: boolean;
  onLimitReached?: (req: any) => void;
}

export interface RateLimitInfo {
  limit: number;
  current: number;
  remaining: number;
  resetTime: Date;
}

class RateLimitStore {
  private store = new Map<string, { count: number; resetTime: number }>();
  
  async get(key: string): Promise<{ count: number; resetTime: number } | null> {
    return this.store.get(key) || null;
  }
  
  async set(key: string, value: { count: number; resetTime: number }): Promise<void> {
    this.store.set(key, value);
  }
  
  async increment(key: string, windowMs: number): Promise<{ count: number; resetTime: number }> {
    const now = Date.now();
    const existing = this.store.get(key);
    
    if (!existing || now > existing.resetTime) {
      // Create new window
      const value = { count: 1, resetTime: now + windowMs };
      this.store.set(key, value);
      return value;
    } else {
      // Increment existing window
      const value = { count: existing.count + 1, resetTime: existing.resetTime };
      this.store.set(key, value);
      return value;
    }
  }
  
  async cleanup(): Promise<void> {
    const now = Date.now();
    for (const [key, value] of this.store.entries()) {
      if (now > value.resetTime) {
        this.store.delete(key);
      }
    }
  }
}

export class RateLimiter {
  private store: RateLimitStore;
  private cleanupInterval: NodeJS.Timeout;
  
  constructor(
    private options: RateLimitOptions,
    store?: RateLimitStore
  ) {
    this.store = store || new RateLimitStore();
    
    // Cleanup expired entries every minute
    this.cleanupInterval = setInterval(() => {
      this.store.cleanup();
    }, 60000);
  }
  
  async checkLimit(identifier: string): Promise<{
    allowed: boolean;
    info: RateLimitInfo;
  }> {
    const result = await this.store.increment(identifier, this.options.windowMs);
    
    const info: RateLimitInfo = {
      limit: this.options.maxRequests,
      current: result.count,
      remaining: Math.max(0, this.options.maxRequests - result.count),
      resetTime: new Date(result.resetTime)
    };
    
    return {
      allowed: result.count <= this.options.maxRequests,
      info
    };
  }
  
  async isLimited(identifier: string): Promise<boolean> {
    const result = await this.checkLimit(identifier);
    return !result.allowed;
  }
  
  async getRemainingRequests(identifier: string): Promise<number> {
    const result = await this.checkLimit(identifier);
    return result.info.remaining;
  }
  
  destroy(): void {
    if (this.cleanupInterval) {
      clearInterval(this.cleanupInterval);
    }
  }
}

/**
 * Create rate limiter middleware for Fastify
 */
export function createRateLimitMiddleware(options: RateLimitOptions) {
  const limiter = new RateLimiter(options);
  
  return async (request: any, reply: any) => {
    const key = options.keyGenerator 
      ? options.keyGenerator(request)
      : request.ip || 'global';
    
    const result = await limiter.checkLimit(key);
    
    // Add rate limit headers
    reply.header('X-RateLimit-Limit', result.info.limit);
    reply.header('X-RateLimit-Remaining', result.info.remaining);
    reply.header('X-RateLimit-Reset', Math.ceil(result.info.resetTime.getTime() / 1000));
    
    if (!result.allowed) {
      if (options.onLimitReached) {
        options.onLimitReached(request);
      }
      
      reply.status(429).send({
        error: {
          code: 'RATE_LIMIT_EXCEEDED',
          message: 'Too many requests, please try again later.',
          retryAfter: Math.ceil((result.info.resetTime.getTime() - Date.now()) / 1000)
        }
      });
      return;
    }
  };
}

/**
 * Rate limiter for function calls
 */
export function rateLimit<T extends (...args: any[]) => any>(
  fn: T,
  maxCalls: number,
  windowMs: number
): T {
  const calls: number[] = [];
  
  return ((...args: Parameters<T>): ReturnType<T> => {
    const now = Date.now();
    
    // Remove old calls outside the window
    while (calls.length > 0 && calls[0] <= now - windowMs) {
      calls.shift();
    }
    
    if (calls.length >= maxCalls) {
      throw new Error(`Rate limit exceeded: ${maxCalls} calls per ${windowMs}ms`);
    }
    
    calls.push(now);
    return fn(...args);
  }) as T;
}
```

### 4. Performance Monitoring

#### server/utils/performance.ts
Port from: `swarm/utils/performance.py`

```typescript
import { performance } from 'perf_hooks';
import { EventEmitter } from 'events';

export interface PerformanceMetric {
  name: string;
  value: number;
  unit: string;
  timestamp: Date;
  labels?: Record<string, string>;
}

export interface TimerMetric extends PerformanceMetric {
  duration: number;
  startTime: number;
  endTime: number;
}

export interface MemoryMetric extends PerformanceMetric {
  heapUsed: number;
  heapTotal: number;
  external: number;
  rss: number;
}

class PerformanceMonitor extends EventEmitter {
  private metrics: PerformanceMetric[] = [];
  private timers: Map<string, number> = new Map();
  private memoryInterval?: NodeJS.Timeout;
  
  constructor() {
    super();
    this.startMemoryMonitoring();
  }
  
  startTimer(name: string, labels?: Record<string, string>): void {
    const key = `${name}:${JSON.stringify(labels || {})}`;
    this.timers.set(key, performance.now());
  }
  
  endTimer(name: string, labels?: Record<string, string>): TimerMetric {
    const key = `${name}:${JSON.stringify(labels || {})}`;
    const startTime = this.timers.get(key);
    
    if (!startTime) {
      throw new Error(`Timer '${name}' was not started`);
    }
    
    const endTime = performance.now();
    const duration = endTime - startTime;
    
    const metric: TimerMetric = {
      name,
      value: duration,
      unit: 'ms',
      timestamp: new Date(),
      labels,
      duration,
      startTime,
      endTime
    };
    
    this.addMetric(metric);
    this.timers.delete(key);
    
    return metric;
  }
  
  measureAsync<T>(
    name: string,
    fn: () => Promise<T>,
    labels?: Record<string, string>
  ): Promise<T> {
    this.startTimer(name, labels);
    
    return fn().finally(() => {
      this.endTimer(name, labels);
    });
  }
  
  measureSync<T>(
    name: string,
    fn: () => T,
    labels?: Record<string, string>
  ): T {
    this.startTimer(name, labels);
    
    try {
      return fn();
    } finally {
      this.endTimer(name, labels);
    }
  }
  
  recordMetric(
    name: string,
    value: number,
    unit: string = 'count',
    labels?: Record<string, string>
  ): PerformanceMetric {
    const metric: PerformanceMetric = {
      name,
      value,
      unit,
      timestamp: new Date(),
      labels
    };
    
    this.addMetric(metric);
    return metric;
  }
  
  getMemoryUsage(): MemoryMetric {
    const memUsage = process.memoryUsage();
    
    return {
      name: 'memory_usage',
      value: memUsage.heapUsed,
      unit: 'bytes',
      timestamp: new Date(),
      heapUsed: memUsage.heapUsed,
      heapTotal: memUsage.heapTotal,
      external: memUsage.external,
      rss: memUsage.rss
    };
  }
  
  getCpuUsage(): PerformanceMetric {
    const cpuUsage = process.cpuUsage();
    const totalUsage = cpuUsage.user + cpuUsage.system;
    
    return {
      name: 'cpu_usage',
      value: totalUsage,
      unit: 'microseconds',
      timestamp: new Date(),
      labels: {
        user: cpuUsage.user.toString(),
        system: cpuUsage.system.toString()
      }
    };
  }
  
  getMetrics(name?: string, since?: Date): PerformanceMetric[] {
    let filtered = this.metrics;
    
    if (name) {
      filtered = filtered.filter(m => m.name === name);
    }
    
    if (since) {
      filtered = filtered.filter(m => m.timestamp >= since);
    }
    
    return filtered;
  }
  
  clearMetrics(): void {
    this.metrics = [];
  }
  
  getStats(name: string, since?: Date): {
    count: number;
    min: number;
    max: number;
    avg: number;
    total: number;
  } {
    const metrics = this.getMetrics(name, since);
    
    if (metrics.length === 0) {
      return { count: 0, min: 0, max: 0, avg: 0, total: 0 };
    }
    
    const values = metrics.map(m => m.value);
    const total = values.reduce((sum, val) => sum + val, 0);
    
    return {
      count: values.length,
      min: Math.min(...values),
      max: Math.max(...values),
      avg: total / values.length,
      total
    };
  }
  
  private addMetric(metric: PerformanceMetric): void {
    this.metrics.push(metric);
    this.emit('metric', metric);
    
    // Keep only last 1000 metrics to prevent memory leaks
    if (this.metrics.length > 1000) {
      this.metrics = this.metrics.slice(-1000);
    }
  }
  
  private startMemoryMonitoring(): void {
    this.memoryInterval = setInterval(() => {
      const memoryMetric = this.getMemoryUsage();
      this.addMetric(memoryMetric);
    }, 30000); // Every 30 seconds
  }
  
  destroy(): void {
    if (this.memoryInterval) {
      clearInterval(this.memoryInterval);
    }
    this.removeAllListeners();
  }
}

// Create default performance monitor
export const performanceMonitor = new PerformanceMonitor();

// Decorator for measuring method performance
export function measure(name?: string, labels?: Record<string, string>) {
  return function (target: any, propertyKey: string, descriptor: PropertyDescriptor) {
    const originalMethod = descriptor.value;
    const metricName = name || `${target.constructor.name}.${propertyKey}`;
    
    descriptor.value = function (...args: any[]) {
      if (originalMethod.constructor.name === 'AsyncFunction') {
        return performanceMonitor.measureAsync(metricName, () => originalMethod.apply(this, args), labels);
      } else {
        return performanceMonitor.measureSync(metricName, () => originalMethod.apply(this, args), labels);
      }
    };
    
    return descriptor;
  };
}

// Helper functions
export function startTimer(name: string, labels?: Record<string, string>): void {
  performanceMonitor.startTimer(name, labels);
}

export function endTimer(name: string, labels?: Record<string, string>): TimerMetric {
  return performanceMonitor.endTimer(name, labels);
}

export function recordMetric(
  name: string,
  value: number,
  unit?: string,
  labels?: Record<string, string>
): PerformanceMetric {
  return performanceMonitor.recordMetric(name, value, unit, labels);
}

export async function measureAsyncOperation<T>(
  name: string,
  operation: () => Promise<T>,
  labels?: Record<string, string>
): Promise<T> {
  return performanceMonitor.measureAsync(name, operation, labels);
}

export function measureSyncOperation<T>(
  name: string,
  operation: () => T,
  labels?: Record<string, string>
): T {
  return performanceMonitor.measureSync(name, operation, labels);
}
```

### 5. Caching Utilities

#### server/utils/cache.ts
```typescript
export interface CacheOptions {
  ttl?: number; // Time to live in milliseconds
  maxSize?: number; // Maximum number of entries
  checkPeriod?: number; // Cleanup interval in milliseconds
}

export interface CacheEntry<T> {
  value: T;
  timestamp: number;
  ttl: number;
  accessed: number;
}

export class Cache<T = any> {
  private store = new Map<string, CacheEntry<T>>();
  private cleanupInterval?: NodeJS.Timeout;
  
  constructor(private options: CacheOptions = {}) {
    const { checkPeriod = 60000 } = options; // Default 1 minute
    
    if (checkPeriod > 0) {
      this.cleanupInterval = setInterval(() => {
        this.cleanup();
      }, checkPeriod);
    }
  }
  
  set(key: string, value: T, ttl?: number): void {
    const entry: CacheEntry<T> = {
      value,
      timestamp: Date.now(),
      ttl: ttl || this.options.ttl || 0,
      accessed: Date.now()
    };
    
    // Remove oldest entries if max size exceeded
    if (this.options.maxSize && this.store.size >= this.options.maxSize) {
      this.evictOldest();
    }
    
    this.store.set(key, entry);
  }
  
  get(key: string): T | null {
    const entry = this.store.get(key);
    
    if (!entry) {
      return null;
    }
    
    // Check if expired
    if (entry.ttl > 0 && Date.now() - entry.timestamp > entry.ttl) {
      this.store.delete(key);
      return null;
    }
    
    // Update access time
    entry.accessed = Date.now();
    
    return entry.value;
  }
  
  has(key: string): boolean {
    return this.get(key) !== null;
  }
  
  delete(key: string): boolean {
    return this.store.delete(key);
  }
  
  clear(): void {
    this.store.clear();
  }
  
  size(): number {
    return this.store.size;
  }
  
  keys(): string[] {
    return Array.from(this.store.keys());
  }
  
  values(): T[] {
    return Array.from(this.store.values()).map(entry => entry.value);
  }
  
  entries(): Array<[string, T]> {
    return Array.from(this.store.entries()).map(([key, entry]) => [key, entry.value]);
  }
  
  // Get or set pattern
  async getOrSet(
    key: string,
    factory: () => Promise<T> | T,
    ttl?: number
  ): Promise<T> {
    const existing = this.get(key);
    if (existing !== null) {
      return existing;
    }
    
    const value = await factory();
    this.set(key, value, ttl);
    return value;
  }
  
  // Memoize a function with caching
  memoize<TArgs extends any[], TReturn>(
    fn: (...args: TArgs) => Promise<TReturn> | TReturn,
    keyGenerator: (...args: TArgs) => string = (...args) => JSON.stringify(args),
    ttl?: number
  ): (...args: TArgs) => Promise<TReturn> {
    return async (...args: TArgs): Promise<TReturn> => {
      const key = keyGenerator(...args);
      return this.getOrSet(key, () => fn(...args), ttl);
    };
  }
  
  private cleanup(): void {
    const now = Date.now();
    
    for (const [key, entry] of this.store.entries()) {
      if (entry.ttl > 0 && now - entry.timestamp > entry.ttl) {
        this.store.delete(key);
      }
    }
  }
  
  private evictOldest(): void {
    let oldestKey = '';
    let oldestTime = Infinity;
    
    for (const [key, entry] of this.store.entries()) {
      if (entry.accessed < oldestTime) {
        oldestTime = entry.accessed;
        oldestKey = key;
      }
    }
    
    if (oldestKey) {
      this.store.delete(oldestKey);
    }
  }
  
  destroy(): void {
    if (this.cleanupInterval) {
      clearInterval(this.cleanupInterval);
    }
    this.clear();
  }
}

// Global cache instances
export const memoryCache = new Cache({
  ttl: 300000, // 5 minutes
  maxSize: 1000,
  checkPeriod: 60000 // 1 minute
});

export const shortTermCache = new Cache({
  ttl: 60000, // 1 minute
  maxSize: 500,
  checkPeriod: 30000 // 30 seconds
});

export const longTermCache = new Cache({
  ttl: 3600000, // 1 hour
  maxSize: 100,
  checkPeriod: 300000 // 5 minutes
});
```

## Testing Strategy

### Unit Tests

#### server/utils/__tests__/async-utils.test.ts
```typescript
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { retry, withTimeout, sleep, batchExecute, TimeoutError, RetryError } from '../async-utils.js';

describe('Async Utils', () => {
  beforeEach(() => {
    vi.clearAllTimers();
    vi.useFakeTimers();
  });
  
  describe('retry', () => {
    it('should succeed on first attempt', async () => {
      const fn = vi.fn().mockResolvedValue('success');
      
      const result = await retry(fn, { maxAttempts: 3 });
      
      expect(result).toBe('success');
      expect(fn).toHaveBeenCalledTimes(1);
    });
    
    it('should retry on failure', async () => {
      const fn = vi.fn()
        .mockRejectedValueOnce(new Error('fail'))
        .mockResolvedValue('success');
      
      const promise = retry(fn, { maxAttempts: 3, baseDelay: 100 });
      
      // Advance timer to trigger retry
      vi.advanceTimersByTime(100);
      
      const result = await promise;
      
      expect(result).toBe('success');
      expect(fn).toHaveBeenCalledTimes(2);
    });
    
    it('should throw RetryError after max attempts', async () => {
      const fn = vi.fn().mockRejectedValue(new Error('fail'));
      
      await expect(retry(fn, { maxAttempts: 2, baseDelay: 100 }))
        .rejects.toThrow(RetryError);
      
      expect(fn).toHaveBeenCalledTimes(2);
    });
  });
  
  describe('withTimeout', () => {
    it('should resolve if promise completes before timeout', async () => {
      const promise = Promise.resolve('success');
      
      const result = await withTimeout(promise, 1000);
      
      expect(result).toBe('success');
    });
    
    it('should reject with TimeoutError if timeout exceeded', async () => {
      const promise = new Promise(resolve => setTimeout(resolve, 2000));
      
      const timeoutPromise = withTimeout(promise, 1000);
      
      vi.advanceTimersByTime(1000);
      
      await expect(timeoutPromise).rejects.toThrow(TimeoutError);
    });
  });
  
  describe('batchExecute', () => {
    it('should execute items in batches', async () => {
      const items = [1, 2, 3, 4, 5];
      const executor = vi.fn().mockImplementation(x => Promise.resolve(x * 2));
      
      const results = await batchExecute(items, executor, 2);
      
      expect(results).toEqual([2, 4, 6, 8, 10]);
      expect(executor).toHaveBeenCalledTimes(5);
    });
  });
});
```

## Migration Checklist

- [ ] Create logging utilities with structured logging
- [ ] Implement async utilities (retry, timeout, debounce, throttle)
- [ ] Add rate limiting utilities and middleware
- [ ] Create performance monitoring and metrics collection
- [ ] Implement caching utilities with TTL and eviction
- [ ] Add validation and sanitization helpers
- [ ] Create error handling and reporting utilities
- [ ] Implement configuration management utilities
- [ ] Add testing helpers and mocks
- [ ] Write comprehensive unit tests for all utilities
- [ ] Add integration tests for performance monitoring
- [ ] Test error scenarios and edge cases
- [ ] Validate performance under load
- [ ] Add documentation and examples

## Best Practices

### 1. Error Handling
- Use typed errors with meaningful messages
- Implement proper error logging
- Provide context for debugging
- Handle edge cases gracefully

### 2. Performance
- Use efficient algorithms and data structures
- Implement caching where appropriate
- Monitor memory usage and clean up resources
- Optimize for common use cases

### 3. Testing
- Write comprehensive unit tests
- Test error scenarios and edge cases
- Use mocks and stubs appropriately
- Test performance characteristics

### 4. Documentation
- Provide clear API documentation
- Include usage examples
- Document performance characteristics
- Explain design decisions

## Next Steps

1. Review [Service Integration](./09-service-integration.md)
2. Implement additional utility functions as needed
3. Add comprehensive monitoring and alerting
4. Set up performance benchmarking
5. Configure automated testing and quality gates