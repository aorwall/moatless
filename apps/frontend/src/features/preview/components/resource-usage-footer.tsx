import type { ResourceMetrics } from '@moatless/api-client'
import { cn } from '@/lib/utils'

function getResourceColor(
  cpuPercent: number | null | undefined,
  loopMs: number | null | undefined,
): string {
  const cpu = cpuPercent ?? 0
  const loop = loopMs ?? 0
  if (cpu > 80 || loop > 1000) {
    return 'text-red-500'
  }
  if (cpu > 50 || loop > 100) {
    return 'text-yellow-500'
  }
  return 'text-muted-foreground'
}

function formatMemory(mb: number): string {
  if (mb >= 1024) {
    return `${(mb / 1024).toFixed(1)} GB`
  }
  return `${Math.round(mb)} MB`
}

/** Compact CPU/memory/event-loop strip shown under the preview iframe. */
export function ResourceUsageFooter({ resources }: { resources: ResourceMetrics }) {
  return (
    <div
      className={cn(
        'h-6 flex-shrink-0 border-t px-3 flex items-center gap-3 text-xs font-mono tabular-nums',
        getResourceColor(resources.cpuUsagePercent, resources.eventLoopLatencyMs),
      )}
    >
      {resources.cpuUsagePercent !== null && <span>CPU {resources.cpuUsagePercent}%</span>}
      <span>
        Mem {formatMemory(resources.memoryUsageMb)}
        {resources.memoryLimitMb != null ? `/${formatMemory(resources.memoryLimitMb)}` : ''}
      </span>
      <span>Loop {Math.round(resources.eventLoopLatencyMs)}ms</span>
    </div>
  )
}
