import type { Task } from '@moatless/types'
import { GitBranch } from 'lucide-react'
import { getTaskStatusConfig } from '@/features/tasks/utils/task-status'

interface TaskStatusIconProps {
  task: Pick<Task, 'status' | 'iterations'>
  showText?: boolean
  className?: string
}

export function TaskStatusIcon({ task, showText = true, className = '' }: TaskStatusIconProps) {
  const statusConfig = getTaskStatusConfig(task.status)

  // Get git status from the last iteration for open/running tasks
  const lastIteration = task.iterations?.[task.iterations.length - 1]
  const gitStatus =
    task.status === 'open' || task.status === 'running' ? lastIteration?.gitStatus : null

  // Determine icon and color based on task status and git conditions
  let icon = statusConfig.icon
  let color = statusConfig.color

  if (gitStatus && (task.status === 'open' || task.status === 'running')) {
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
  } else if (task.status === 'open' || task.status === 'running') {
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
