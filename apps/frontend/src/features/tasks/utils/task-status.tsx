import type { Task } from '@moatless/types'
import { AlertCircle, CheckCircle, CircleDot, GitMerge, Loader2, PlayCircle } from 'lucide-react'
import type { ReactElement } from 'react'

export interface TaskStatusConfig {
  icon: ReactElement
  tooltip: string
  color: string
}

/**
 * Determines if a task is currently running based on iteration status.
 * Note: This is a temporary implementation. The actual running state
 * should come from workspace status when available.
 */
export function isTaskRunning(task: Pick<Task, 'iterations'>): boolean {
  const lastIteration = task.iterations?.[task.iterations.length - 1]
  return lastIteration?.status === 'active'
}

/**
 * Gets the display status for a task.
 * Note: This is a temporary implementation. The actual running state
 * should come from workspace status when available.
 */
export function getTaskDisplayStatus(task: Pick<Task, 'status' | 'iterations'>): string {
  // If last iteration is active, show as running regardless of task status
  if (isTaskRunning(task)) {
    return 'running'
  }
  // Otherwise use the task status
  return task.status
}

export function getTaskStatusConfig(status: string): TaskStatusConfig {
  switch (status) {
    case 'open':
      return {
        icon: <PlayCircle className="h-3 w-3" />,
        tooltip: 'Open',
        color: 'text-status-info',
      }
    case 'running':
      return {
        icon: <Loader2 className="h-3 w-3 animate-spin" />,
        tooltip: 'Running',
        color: 'text-status-warning',
      }
    case 'error':
      return {
        icon: <AlertCircle className="h-3 w-3" />,
        tooltip: 'Error',
        color: 'text-status-error',
      }
    case 'merged':
      return {
        icon: <GitMerge className="h-3 w-3" />,
        tooltip: 'Merged',
        color: 'text-status-success',
      }
    case 'closed':
      return {
        icon: <CircleDot className="h-3 w-3" />,
        tooltip: 'Closed',
        color: 'text-status-pending',
      }
    // Legacy statuses for backward compatibility
    case 'active':
      return {
        icon: <PlayCircle className="h-3 w-3" />,
        tooltip: 'Open',
        color: 'text-status-info',
      }
    case 'completed':
      return {
        icon: <CheckCircle className="h-3 w-3" />,
        tooltip: 'Completed',
        color: 'text-status-success',
      }
    default:
      return {
        icon: <CircleDot className="h-3 w-3" />,
        tooltip: 'Unknown',
        color: 'text-status-pending',
      }
  }
}

export function getTaskStatusIcon(status: string, className?: string): ReactElement {
  const config = getTaskStatusConfig(status)
  const iconClass = className || `${config.color}`

  // Clone the icon element with the new className
  return <span className={iconClass}>{config.icon}</span>
}
