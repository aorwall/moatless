import { EventEmitter } from 'node:events'
import { performance } from 'node:perf_hooks'

export interface PerformanceMetric {
  name: string
  value: number
  unit: string
  timestamp: Date
  labels?: Record<string, string>
}

export interface TimerMetric extends PerformanceMetric {
  duration: number
  startTime: number
  endTime: number
}

export interface MemoryMetric extends PerformanceMetric {
  heapUsed: number
  heapTotal: number
  external: number
  rss: number
}

class PerformanceMonitor extends EventEmitter {
  private metrics: PerformanceMetric[] = []
  private timers: Map<string, number> = new Map()
  private memoryInterval?: NodeJS.Timeout

  constructor() {
    super()
    this.startMemoryMonitoring()
  }

  startTimer(name: string, labels?: Record<string, string>): void {
    const key = `${name}:${JSON.stringify(labels || {})}`
    this.timers.set(key, performance.now())
  }

  endTimer(name: string, labels?: Record<string, string>): TimerMetric {
    const key = `${name}:${JSON.stringify(labels || {})}`
    const startTime = this.timers.get(key)

    if (!startTime) {
      throw new Error(`Timer '${name}' was not started`)
    }

    const endTime = performance.now()
    const duration = endTime - startTime

    const metric: TimerMetric = {
      name,
      value: duration,
      unit: 'ms',
      timestamp: new Date(),
      labels,
      duration,
      startTime,
      endTime,
    }

    this.addMetric(metric)
    this.timers.delete(key)

    return metric
  }

  measureAsync<T>(name: string, fn: () => Promise<T>, labels?: Record<string, string>): Promise<T> {
    this.startTimer(name, labels)

    return fn().finally(() => {
      this.endTimer(name, labels)
    })
  }

  measureSync<T>(name: string, fn: () => T, labels?: Record<string, string>): T {
    this.startTimer(name, labels)

    try {
      return fn()
    } finally {
      this.endTimer(name, labels)
    }
  }

  recordMetric(
    name: string,
    value: number,
    unit: string = 'count',
    labels?: Record<string, string>,
  ): PerformanceMetric {
    const metric: PerformanceMetric = {
      name,
      value,
      unit,
      timestamp: new Date(),
      labels,
    }

    this.addMetric(metric)
    return metric
  }

  getMemoryUsage(): MemoryMetric {
    const memUsage = process.memoryUsage()

    return {
      name: 'memory_usage',
      value: memUsage.heapUsed,
      unit: 'bytes',
      timestamp: new Date(),
      heapUsed: memUsage.heapUsed,
      heapTotal: memUsage.heapTotal,
      external: memUsage.external,
      rss: memUsage.rss,
    }
  }

  getCpuUsage(): PerformanceMetric {
    const cpuUsage = process.cpuUsage()
    const totalUsage = cpuUsage.user + cpuUsage.system

    return {
      name: 'cpu_usage',
      value: totalUsage,
      unit: 'microseconds',
      timestamp: new Date(),
      labels: {
        user: cpuUsage.user.toString(),
        system: cpuUsage.system.toString(),
      },
    }
  }

  getMetrics(name?: string, since?: Date): PerformanceMetric[] {
    let filtered = this.metrics

    if (name) {
      filtered = filtered.filter((m) => m.name === name)
    }

    if (since) {
      filtered = filtered.filter((m) => m.timestamp >= since)
    }

    return filtered
  }

  clearMetrics(): void {
    this.metrics = []
  }

  getStats(
    name: string,
    since?: Date,
  ): {
    count: number
    min: number
    max: number
    avg: number
    total: number
  } {
    const metrics = this.getMetrics(name, since)

    if (metrics.length === 0) {
      return { count: 0, min: 0, max: 0, avg: 0, total: 0 }
    }

    const values = metrics.map((m) => m.value)
    const total = values.reduce((sum, val) => sum + val, 0)

    return {
      count: values.length,
      min: Math.min(...values),
      max: Math.max(...values),
      avg: total / values.length,
      total,
    }
  }

  private addMetric(metric: PerformanceMetric): void {
    this.metrics.push(metric)
    this.emit('metric', metric)

    // Keep only last 1000 metrics to prevent memory leaks
    if (this.metrics.length > 1000) {
      this.metrics = this.metrics.slice(-1000)
    }
  }

  private startMemoryMonitoring(): void {
    this.memoryInterval = setInterval(() => {
      const memoryMetric = this.getMemoryUsage()
      this.addMetric(memoryMetric)
    }, 30000) // Every 30 seconds
  }

  destroy(): void {
    if (this.memoryInterval) {
      clearInterval(this.memoryInterval)
    }
    this.removeAllListeners()
  }
}

// Create default performance monitor
export const performanceMonitor = new PerformanceMonitor()

// Decorator for measuring method performance
export function measure(name?: string, labels?: Record<string, string>) {
  return (target: unknown, propertyKey: string, descriptor: PropertyDescriptor) => {
    const originalMethod = descriptor.value
    const metricName = name || `${target.constructor.name}.${propertyKey}`

    descriptor.value = function (...args: unknown[]) {
      if (originalMethod.constructor.name === 'AsyncFunction') {
        return performanceMonitor.measureAsync(
          metricName,
          () => originalMethod.apply(this, args),
          labels,
        )
      } else {
        return performanceMonitor.measureSync(
          metricName,
          () => originalMethod.apply(this, args),
          labels,
        )
      }
    }

    return descriptor
  }
}

// Helper functions
export function startTimer(name: string, labels?: Record<string, string>): void {
  performanceMonitor.startTimer(name, labels)
}

export function endTimer(name: string, labels?: Record<string, string>): TimerMetric {
  return performanceMonitor.endTimer(name, labels)
}

export function recordMetric(
  name: string,
  value: number,
  unit?: string,
  labels?: Record<string, string>,
): PerformanceMetric {
  return performanceMonitor.recordMetric(name, value, unit, labels)
}

export async function measureAsyncOperation<T>(
  name: string,
  operation: () => Promise<T>,
  labels?: Record<string, string>,
): Promise<T> {
  return performanceMonitor.measureAsync(name, operation, labels)
}

export function measureSyncOperation<T>(
  name: string,
  operation: () => T,
  labels?: Record<string, string>,
): T {
  return performanceMonitor.measureSync(name, operation, labels)
}
