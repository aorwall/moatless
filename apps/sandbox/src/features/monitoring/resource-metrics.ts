import { readFile } from 'node:fs/promises'
import { logger } from '../../shared/logger'

interface ResourceMetrics {
  memoryUsageMb: number
  memoryLimitMb: number | null
  memoryUsagePercent: number | null
  cpuUsagePercent: number | null
  eventLoopLatencyMs: number
  uptimeSeconds: number
}

let latestMetrics: ResourceMetrics | null = null
let monitorInterval: ReturnType<typeof setInterval> | null = null
let emitCallback: ((metrics: ResourceMetrics) => void) | null = null

// CPU tracking state
let lastCpuUsageUsec = 0
let lastCpuSampleTime = 0

const SAMPLE_INTERVAL_MS = 5000

async function readCgroupFile(path: string): Promise<string | null> {
  try {
    return (await readFile(path, 'utf-8')).trim()
  } catch {
    return null
  }
}

async function readMemoryUsageBytes(): Promise<number | null> {
  const val = await readCgroupFile('/sys/fs/cgroup/memory.current')
  if (val === null) {
    return null
  }
  const parsed = Number.parseInt(val, 10)
  return Number.isNaN(parsed) ? null : parsed
}

async function readMemoryLimitBytes(): Promise<number | null> {
  const val = await readCgroupFile('/sys/fs/cgroup/memory.max')
  if (val === null || val === 'max') {
    return null
  }
  const parsed = Number.parseInt(val, 10)
  return Number.isNaN(parsed) ? null : parsed
}

async function readCpuUsageUsec(): Promise<number | null> {
  const content = await readCgroupFile('/sys/fs/cgroup/cpu.stat')
  if (content === null) {
    return null
  }
  for (const line of content.split('\n')) {
    if (line.startsWith('usage_usec ')) {
      const parsed = Number.parseInt(line.split(' ')[1], 10)
      return Number.isNaN(parsed) ? null : parsed
    }
  }
  return null
}

function computeCpuPercent(currentUsageUsec: number, now: number): number | null {
  if (lastCpuSampleTime === 0) {
    lastCpuUsageUsec = currentUsageUsec
    lastCpuSampleTime = now
    return null
  }

  const elapsedMs = now - lastCpuSampleTime
  if (elapsedMs <= 0) {
    return null
  }

  const deltaUsec = currentUsageUsec - lastCpuUsageUsec
  // Convert: deltaUsec / (elapsedMs * 1000) gives fraction of 1 core
  // Multiply by 100 for percentage of 1 core
  const cpuPercent = (deltaUsec / (elapsedMs * 1000)) * 100

  lastCpuUsageUsec = currentUsageUsec
  lastCpuSampleTime = now

  return Math.round(cpuPercent * 10) / 10
}

async function sampleMetrics(expectedIntervalMs: number, lastSampleTime: number): Promise<void> {
  const now = Date.now()

  // Event loop latency: difference between expected and actual interval
  const eventLoopLatencyMs =
    lastSampleTime > 0 ? Math.max(0, now - lastSampleTime - expectedIntervalMs) : 0

  // Memory
  const memUsageBytes = await readMemoryUsageBytes()
  const memLimitBytes = await readMemoryLimitBytes()
  const memoryUsageMb =
    memUsageBytes !== null ? Math.round((memUsageBytes / 1024 / 1024) * 10) / 10 : 0
  const memoryLimitMb =
    memLimitBytes !== null ? Math.round((memLimitBytes / 1024 / 1024) * 10) / 10 : null
  const memoryUsagePercent =
    memoryLimitMb !== null && memoryLimitMb > 0
      ? Math.round((memoryUsageMb / memoryLimitMb) * 1000) / 10
      : null

  // CPU
  const cpuUsageUsec = await readCpuUsageUsec()
  const cpuUsagePercent = cpuUsageUsec !== null ? computeCpuPercent(cpuUsageUsec, now) : null

  const uptimeSeconds = Math.round(process.uptime())

  latestMetrics = {
    memoryUsageMb,
    memoryLimitMb,
    memoryUsagePercent,
    cpuUsagePercent,
    eventLoopLatencyMs,
    uptimeSeconds,
  }

  if (emitCallback && latestMetrics) {
    emitCallback(latestMetrics)
  }

  // Warn on high resource usage
  if (cpuUsagePercent !== null && cpuUsagePercent > 80) {
    logger.warn({ cpuUsagePercent, eventLoopLatencyMs }, 'High CPU usage detected')
  }
  if (eventLoopLatencyMs > 1000) {
    logger.warn({ eventLoopLatencyMs, cpuUsagePercent }, 'High event loop latency detected')
  }
}

export function initResourceMonitor(options?: {
  onMetrics?: (metrics: ResourceMetrics) => void
}): void {
  if (monitorInterval) {
    return
  }

  emitCallback = options?.onMetrics ?? null

  let lastSampleTime = 0

  // Take an initial sample immediately
  sampleMetrics(SAMPLE_INTERVAL_MS, 0).then(() => {
    lastSampleTime = Date.now()
  })

  monitorInterval = setInterval(() => {
    const capturedLastSampleTime = lastSampleTime
    lastSampleTime = Date.now()
    sampleMetrics(SAMPLE_INTERVAL_MS, capturedLastSampleTime)
  }, SAMPLE_INTERVAL_MS)

  // Don't prevent process from exiting
  monitorInterval.unref()
}

export function getResourceMetrics(): ResourceMetrics | null {
  return latestMetrics
}
