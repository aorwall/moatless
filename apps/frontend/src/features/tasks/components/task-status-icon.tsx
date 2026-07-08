import type { SandboxStatusFullResponse, TaskResponse } from '@moatless/api-client'
import { AlertCircle, CheckCircle, Clock, XCircle } from 'lucide-react'
import {
  getTaskActivityConfig,
  getTaskStatusConfig,
  type TaskActivityState,
} from '@/features/tasks/utils/task-status'

interface TaskStatusIconProps {
  task: Pick<TaskResponse, 'status'>
  sandbox?: SandboxStatusFullResponse
  /**
   * Live activity derived from the event stream. When set to a non-idle state
   * it overrides the lifecycle status icon (e.g. a spinner while the agent
   * works, an hourglass while the sandbox starts up).
   */
  activityState?: TaskActivityState
  showText?: boolean
  className?: string
}

export function TaskStatusIcon({
  task,
  sandbox,
  activityState,
  showText = true,
  className = '',
}: TaskStatusIconProps) {
  const statusConfig = getTaskStatusConfig(task.status)
  const activityConfig = activityState ? getTaskActivityConfig(activityState) : null

  const environmentStatus = sandbox?.environmentStatus

  // Prefer the live activity indicator (agent working / starting up / sandbox
  // running); otherwise fall back to the lifecycle status icon.
  let icon = activityConfig?.icon ?? statusConfig.icon
  let color = activityConfig?.color ?? statusConfig.color
  let displayText: string = activityConfig?.tooltip ?? task.status

  // Override for environment issues (show these as higher priority)
  if (environmentStatus === 'error') {
    icon = <XCircle className="h-3.5 w-3.5" />
    color = 'text-red-500'
    displayText = 'env error'
  } else if (environmentStatus === 'removed') {
    icon = <AlertCircle className="h-3.5 w-3.5" />
    color = 'text-muted-foreground'
    displayText = 'removed'
  } else if (environmentStatus === 'not_created') {
    icon = <AlertCircle className="h-3.5 w-3.5" />
    color = 'text-orange-500'
    displayText = 'not setup'
  } else if (environmentStatus === 'initializing') {
    icon = <Clock className="h-3.5 w-3.5" />
    color = 'text-yellow-500'
    displayText = 'initializing'
  } else if (task.status === 'open' && environmentStatus === 'ready') {
    // Task is open and environment is ready
    icon = <CheckCircle className="h-3.5 w-3.5" />
    color = 'text-green-500'
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
