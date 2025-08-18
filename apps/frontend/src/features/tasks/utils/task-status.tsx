import type { Task } from '@moatless/types'
import { AlertCircle, CheckCircle, CircleDot, GitMerge, Loader2, PlayCircle } from 'lucide-react'
import type { ReactElement } from 'react'

export interface TaskStatusConfig {
  icon: ReactElement
  tooltip: string
  color: string
}

/**
 * Determines if a task is currently running based on task status or iteration status.
 * Primary check is task.status === 'running', with fallback to iteration status for compatibility.
 */
export function isTaskRunning(task: Pick<Task, 'status' | 'iterations'>): boolean {
  // Primary check: if task has running status, it's running
  if (task.status === 'running') {
    return true
  }

  // Fallback check: check if last iteration is active (for backwards compatibility)
  const lastIteration = task.iterations?.[task.iterations.length - 1]
  return lastIteration?.status === 'active'
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
