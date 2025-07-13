import type { GitSyncStatus } from '@moatless/types/types/api'
import type { Task } from '@moatless/types/types/task'
import { getGitSyncStatusConfig } from '@/features/tasks/utils/git-sync-status'
import { getTaskStatusConfig } from '@/features/tasks/utils/task-status'

interface TaskStatusIconProps {
  task: Pick<Task, 'status' | 'iterations'>
  showText?: boolean
  className?: string
}

export function TaskStatusIcon({ task, showText = true, className = '' }: TaskStatusIconProps) {
  const statusConfig = getTaskStatusConfig(task.status)

  // Get git sync status from the last iteration for open tasks
  const lastIteration = task.iterations?.[task.iterations.length - 1]
  const gitStatus = task.status === 'open' ? lastIteration?.gitStatus : null
  const gitSyncConfig = gitStatus ? getGitSyncStatusConfig(gitStatus.status as GitSyncStatus) : null

  // Use git sync icon/color for open tasks if available, otherwise use task status
  const displayConfig = task.status === 'open' && gitSyncConfig ? gitSyncConfig : statusConfig

  return (
    <div className={`flex items-center gap-1 ${displayConfig.color} ${className}`}>
      {displayConfig.icon}
      {showText && <span>{task.status}</span>}
    </div>
  )
}
