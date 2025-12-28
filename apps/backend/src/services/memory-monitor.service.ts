import { logger } from '../utils/logger'

interface MemoryMetrics {
  heapUsed: number
  heapTotal: number
  rss: number
  external: number
  arrayBuffers: number
  heapUsedMB: number
  rssMB: number
  timestamp: string
}

/**
 * Memory monitoring service for tracking memory usage patterns.
 * Designed to help debug OOMKilled issues in Kubernetes.
 *
 * Environment Variables:
 * - MEMORY_LOGGING_ENABLED: 'true'/'false' to toggle periodic logging (default: 'false')
 * - MEMORY_LOGGING_INTERVAL_MS: interval in ms (default: 60000)
 * - MEMORY_WARNING_THRESHOLD_MB: warn when RSS exceeds this (default: 3500)
 */
class MemoryMonitorService {
  private intervalId: ReturnType<typeof setInterval> | null = null
  private previousHeap = 0

  private get isEnabled(): boolean {
    return process.env['MEMORY_LOGGING_ENABLED'] === 'true'
  }

  private get intervalMs(): number {
    return Number(process.env['MEMORY_LOGGING_INTERVAL_MS']) || 60000
  }

  private get warningThresholdMB(): number {
    return Number(process.env['MEMORY_WARNING_THRESHOLD_MB']) || 3500
  }

  /**
   * Start memory monitoring.
   * Always logs startup memory as a baseline.
   * Periodic logging only runs when MEMORY_LOGGING_ENABLED=true.
   */
  start(): void {
    // Always log startup memory (useful baseline)
    this.logMemory('startup')

    if (this.isEnabled) {
      logger.info({ intervalMs: this.intervalMs }, 'Memory logging enabled')
      this.intervalId = setInterval(() => this.logMemory('periodic'), this.intervalMs)
    }
  }

  /**
   * Stop memory monitoring.
   * Called during graceful shutdown.
   */
  stop(): void {
    if (this.intervalId) {
      clearInterval(this.intervalId)
      this.intervalId = null
    }
  }

  /**
   * Get current memory metrics.
   * Useful for health endpoint and on-demand queries.
   */
  getMetrics(): MemoryMetrics {
    const mem = process.memoryUsage()
    return {
      heapUsed: mem.heapUsed,
      heapTotal: mem.heapTotal,
      rss: mem.rss,
      external: mem.external,
      arrayBuffers: mem.arrayBuffers,
      heapUsedMB: Math.round(mem.heapUsed / 1024 / 1024),
      rssMB: Math.round(mem.rss / 1024 / 1024),
      timestamp: new Date().toISOString(),
    }
  }

  private logMemory(trigger: string): void {
    const metrics = this.getMetrics()
    const heapDelta = metrics.heapUsed - this.previousHeap
    this.previousHeap = metrics.heapUsed

    const logData = {
      trigger,
      heapUsedMB: metrics.heapUsedMB,
      heapTotalMB: Math.round(metrics.heapTotal / 1024 / 1024),
      rssMB: metrics.rssMB,
      externalMB: Math.round(metrics.external / 1024 / 1024),
      arrayBuffersMB: Math.round(metrics.arrayBuffers / 1024 / 1024),
      heapDeltaMB: Math.round(heapDelta / 1024 / 1024),
    }

    // Always warn when approaching limit, regardless of toggle
    if (metrics.rssMB > this.warningThresholdMB) {
      logger.warn(logData, 'Memory warning: approaching limit')
    } else {
      logger.info(logData, 'Memory metrics')
    }
  }
}

export const memoryMonitor = new MemoryMonitorService()
