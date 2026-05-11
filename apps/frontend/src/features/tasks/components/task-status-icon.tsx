import type { SandboxStatusFullResponse, TaskResponse } from '@moatless/api-client'
import { AlertCircle, CheckCircle, Clock, XCircle } from 'lucide-react'
import { getTaskStatusConfig } from '@/features/tasks/utils/task-status'

interface TaskStatusIconProps {
  task: Pick<TaskResponse, 'status'>
  sandbox?: SandboxStatusFullResponse
  showText?: boolean
  className?: string
}

export function TaskStatusIcon({
  task,
  sandbox,
  showText = true,
  className = '',
}: TaskStatusIconProps) {
  const statusConfig = getTaskStatusConfig(task.status)

  const environmentStatus = sandbox?.environmentStatus

  // Determine icon and color based on task status primarily
  let icon = statusConfig.icon
  let color = statusConfig.color
  let displayText: string = task.status

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
