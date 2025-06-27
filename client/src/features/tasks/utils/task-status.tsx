import { AlertCircle, CheckCircle, CircleDot, GitMerge, Loader2, PlayCircle } from 'lucide-react'
import type { ReactElement } from 'react'

export interface TaskStatusConfig {
  icon: ReactElement
  tooltip: string
  color: string
}

export function getTaskStatusConfig(status: string): TaskStatusConfig {
  switch (status) {
    case 'open':
      return {
        icon: <PlayCircle className="h-3 w-3" />,
        tooltip: 'Open',
        color: 'text-blue-600 dark:text-blue-400',
      }
    case 'running':
      return {
        icon: <Loader2 className="h-3 w-3 animate-spin" />,
        tooltip: 'Running',
        color: 'text-yellow-600 dark:text-yellow-400',
      }
    case 'error':
      return {
        icon: <AlertCircle className="h-3 w-3" />,
        tooltip: 'Error',
        color: 'text-red-600 dark:text-red-400',
      }
    case 'merged':
      return {
        icon: <GitMerge className="h-3 w-3" />,
        tooltip: 'Merged',
        color: 'text-green-600 dark:text-green-400',
      }
    case 'closed':
      return {
        icon: <CircleDot className="h-3 w-3" />,
        tooltip: 'Closed',
        color: 'text-gray-600 dark:text-gray-400',
      }
    // Legacy statuses for backward compatibility
    case 'active':
      return {
        icon: <PlayCircle className="h-3 w-3" />,
        tooltip: 'Open',
        color: 'text-blue-600 dark:text-blue-400',
      }
    case 'completed':
      return {
        icon: <CheckCircle className="h-3 w-3" />,
        tooltip: 'Completed',
        color: 'text-green-600 dark:text-green-400',
      }
    default:
      return {
        icon: <CircleDot className="h-3 w-3" />,
        tooltip: 'Unknown',
        color: 'text-gray-600 dark:text-gray-400',
      }
  }
}

export function getTaskStatusIcon(status: string, className?: string): ReactElement {
  const config = getTaskStatusConfig(status)
  const iconClass = className || `${config.color}`

  // Clone the icon element with the new className
  return <span className={iconClass}>{config.icon}</span>
}
