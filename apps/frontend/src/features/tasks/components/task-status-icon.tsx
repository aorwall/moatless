import type { Task, WorkspaceStatusResponse } from '@moatless/types'
import { GitBranch } from 'lucide-react'
import { getTaskStatusConfig } from '@/features/tasks/utils/task-status'

interface TaskStatusIconProps {
  task: Pick<Task, 'status' | 'iterations'>
  workspace?: WorkspaceStatusResponse
  showText?: boolean
  className?: string
}

export function TaskStatusIcon({
  task,
  workspace,
  showText = true,
  className = '',
}: TaskStatusIconProps) {
  const statusConfig = getTaskStatusConfig(task.status)

  // Determine if task is running based on workspace agent state
  const isRunning = workspace?.isRunning ?? false

  // Get git status from workspace if available
  const gitStatus = workspace?.gitStatus

  // Determine icon and color based on task status and git conditions
  let icon = statusConfig.icon
  let color = statusConfig.color

  if (gitStatus && (task.status === 'open' || isRunning)) {
    // Use git branch icon for open/running tasks
    icon = <GitBranch className="h-3.5 w-3.5" />

    // Determine color based on git conditions
    if (gitStatus.hasConflicts) {
      color = 'text-red-500'
    } else if (gitStatus.commitsAhead > 0 || gitStatus.hasUncommittedChanges) {
      color = 'text-yellow-500'
    } else {
      // Clean state - no changes, no conflicts, not ahead
      color = 'text-green-500'
    }
  } else if (task.status === 'open' || isRunning) {
    // No git status available, but task is open/running - show yellow as default
    icon = <GitBranch className="h-3.5 w-3.5" />
    color = 'text-yellow-500'
  }

  return (
    <div className={`flex items-center gap-1 ${color} ${className}`}>
      {icon}
      {showText && <span>{task.status}</span>}
    </div>
  )
}
