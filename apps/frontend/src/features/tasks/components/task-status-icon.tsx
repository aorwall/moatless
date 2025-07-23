import type { Task, WorkspaceStatusResponse } from '@moatless/types'
import {
  AlertCircle,
  CheckCircle,
  Clock,
  GitBranch,
  Loader2,
  PlayCircle,
  XCircle,
} from 'lucide-react'
import { getTaskStatusConfig } from '@/features/tasks/utils/task-status'
import { Badge } from '@/lib/components/ui/badge'
import { Popover, PopoverContent, PopoverTrigger } from '@/lib/components/ui/popover'

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
  const agentState = workspace?.agentState
  const environmentStatus = workspace?.environmentStatus

  // Get git status from workspace if available
  const gitStatus = workspace?.gitStatus

  // Determine icon and color based on task status and workspace conditions
  let icon = statusConfig.icon
  let color = statusConfig.color
  let displayText: string = task.status

  // Override for running states
  if (isRunning || agentState === 'running') {
    icon = <Loader2 className="h-3.5 w-3.5 animate-spin" />
    color = 'text-blue-500'
    displayText = 'running'
  } else if (environmentStatus === 'initializing') {
    icon = <Clock className="h-3.5 w-3.5" />
    color = 'text-yellow-500'
    displayText = 'initializing'
  } else if (environmentStatus === 'error') {
    icon = <XCircle className="h-3.5 w-3.5" />
    color = 'text-red-500'
    displayText = 'env error'
  } else if (environmentStatus === 'not_created') {
    icon = <AlertCircle className="h-3.5 w-3.5" />
    color = 'text-orange-500'
    displayText = 'not setup'
  } else if (gitStatus && (task.status === 'open' || isRunning)) {
    // Use git branch icon for open/running tasks with git status
    icon = <GitBranch className="h-3.5 w-3.5" />

    // Determine color based on git conditions
    if (gitStatus.hasConflicts) {
      color = 'text-red-500'
      displayText = 'conflicts'
    } else if (gitStatus.commitsAhead > 0 || gitStatus.hasUncommittedChanges) {
      color = 'text-yellow-500'
      displayText = 'changes'
    } else {
      // Clean state - no changes, no conflicts, not ahead
      color = 'text-green-500'
      displayText = 'clean'
    }
  } else if (task.status === 'open' && environmentStatus === 'ready') {
    // Task is open and environment is ready
    icon = <CheckCircle className="h-3.5 w-3.5" />
    color = 'text-green-500'
    displayText = 'ready'
  } else if (task.status === 'open') {
    // No git status available, but task is open - show default open state
    icon = <PlayCircle className="h-3.5 w-3.5" />
    color = 'text-blue-500'
  }

  const WorkspaceStatusPopup = () => {
    if (!workspace) return null

    return (
      <div className="space-y-3 w-64">
        <div>
          <h4 className="font-medium text-sm mb-2">Workspace Status</h4>
          <div className="space-y-2 text-xs">
            <div className="flex justify-between">
              <span className="text-muted-foreground">Environment:</span>
              <Badge
                variant={workspace.environmentStatus === 'ready' ? 'default' : 'secondary'}
                className="h-5 text-xs"
              >
                {workspace.environmentStatus}
              </Badge>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Agent:</span>
              <Badge
                variant={workspace.agentState === 'running' ? 'default' : 'outline'}
                className="h-5 text-xs"
              >
                {workspace.agentState}
              </Badge>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Workspace:</span>
              <Badge
                variant={workspace.workspaceState === 'exists' ? 'default' : 'destructive'}
                className="h-5 text-xs"
              >
                {workspace.workspaceState}
              </Badge>
            </div>
          </div>
        </div>

        {gitStatus && (
          <div>
            <h4 className="font-medium text-sm mb-2">Git Status</h4>
            <div className="space-y-1 text-xs">
              {gitStatus.commitsAhead > 0 && (
                <div className="text-blue-600">• {gitStatus.commitsAhead} commits ahead</div>
              )}
              {gitStatus.commitsBehind > 0 && (
                <div className="text-orange-600">• {gitStatus.commitsBehind} commits behind</div>
              )}
              {gitStatus.hasUncommittedChanges && (
                <div className="text-yellow-600">• Uncommitted changes</div>
              )}
              {gitStatus.hasConflicts && <div className="text-red-600">• Conflicts detected</div>}
              {!gitStatus.commitsAhead &&
                !gitStatus.commitsBehind &&
                !gitStatus.hasUncommittedChanges &&
                !gitStatus.hasConflicts && (
                  <div className="text-green-600">• Clean working directory</div>
                )}
            </div>
          </div>
        )}

        {workspace.environmentError && (
          <div>
            <h4 className="font-medium text-sm mb-2 text-red-600">Error</h4>
            <div className="text-xs text-red-600 bg-red-50 p-2 rounded">
              {workspace.environmentError}
            </div>
          </div>
        )}
      </div>
    )
  }

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          className={`flex items-center gap-1 ${color} ${className} hover:opacity-80 transition-opacity cursor-pointer`}
        >
          {icon}
          {showText && <span>{displayText}</span>}
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-auto p-3" align="start">
        <WorkspaceStatusPopup />
      </PopoverContent>
    </Popover>
  )
}
