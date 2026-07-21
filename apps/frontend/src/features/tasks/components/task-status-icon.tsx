import type { LiveStatus, TaskResponse } from '@moatless/api-client'
import { AlertCircle, CheckCircle, Clock, XCircle } from 'lucide-react'
import {
  getTaskActivityConfig,
  getTaskStatusConfig,
  type TaskActivityState,
} from '@/features/tasks/utils/task-status'

interface TaskStatusIconProps {
  task: Pick<TaskResponse, 'status'>
  liveStatus?: Pick<LiveStatus, 'sandboxStatus'> | undefined
  /**
   * Live activity derived from the event stream. When set to a non-idle state
   * it overrides the lifecycle status icon (e.g. a spinner while the agent
   * works, an hourglass while the sandbox starts up).
   */
  activityState?: TaskActivityState | undefined
  showText?: boolean | undefined
  className?: string | undefined
}

export function TaskStatusIcon({
  task,
  liveStatus,
  activityState,
  showText = true,
  className = '',
}: TaskStatusIconProps) {
  const statusConfig = getTaskStatusConfig(task.status)
  const activityConfig = activityState ? getTaskActivityConfig(activityState) : null

  const sandboxStatus = liveStatus?.sandboxStatus

  // Prefer the live activity indicator (agent working / starting up / sandbox
  // running); otherwise fall back to the lifecycle status icon.
  let icon = activityConfig?.icon ?? statusConfig.icon
  let color = activityConfig?.color ?? statusConfig.color
  let displayText: string = activityConfig?.tooltip ?? task.status

  // Override for environment issues (show these as higher priority)
  if (sandboxStatus === 'error') {
    icon = <XCircle className="h-3.5 w-3.5" />
    color = 'text-status-error'
    displayText = 'env error'
  } else if (sandboxStatus === 'removed') {
    icon = <AlertCircle className="h-3.5 w-3.5" />
    color = 'text-muted-foreground'
    displayText = 'removed'
  } else if (sandboxStatus === 'not_created') {
    icon = <AlertCircle className="h-3.5 w-3.5" />
    color = 'text-status-warning'
    displayText = 'not setup'
  } else if (sandboxStatus === 'initializing') {
    icon = <Clock className="h-3.5 w-3.5" />
    color = 'text-status-warning'
    displayText = 'initializing'
  } else if (task.status === 'open' && sandboxStatus === 'ready') {
    // Task is open and environment is ready
    icon = <CheckCircle className="h-3.5 w-3.5" />
    color = 'text-status-success'
    displayText = 'ready'
  }
  // For other statuses (running, merged, closed, etc.), use the default from statusConfig

  return (
    <span className={`flex items-center gap-1 ${color} ${className}`}>
      {icon}
      {showText && <span>{displayText}</span>}
    </span>
  )
}
