import type { Task, WorkspaceStatusResponse } from '@moatless/types'
import { AlertCircle, CheckCircle, Clock, GitBranch, XCircle } from 'lucide-react'
import { getTaskStatusConfig } from '@/features/tasks/utils/task-status'
import { Badge } from '@/lib/components/ui/badge'
import { Popover, PopoverContent, PopoverTrigger } from '@/lib/components/ui/popover'

type WorkspaceGitStatus = WorkspaceStatusResponse['gitStatus']

interface WorkspaceStatusPopupProps {
  totalTurns: number
  totalCostUsd: number
  contextTokens?: number
  workspace?: WorkspaceStatusResponse
  gitStatus?: WorkspaceGitStatus
}

function WorkspaceStatusPopup({
  totalTurns,
  totalCostUsd,
  contextTokens,
  workspace,
  gitStatus,
}: WorkspaceStatusPopupProps) {
  return (
    <div className="space-y-3 w-64">
      <div>
        <h4 className="font-medium text-sm mb-2">Task Statistics</h4>
        <div className="space-y-2 text-xs">
          <div className="flex justify-between">
            <span className="text-muted-foreground">Total Turns:</span>
            <Badge variant="outline" className="h-5 text-xs">
              {totalTurns}
            </Badge>
          </div>
          <div className="flex justify-between">
            <span className="text-muted-foreground">Total Cost:</span>
            <Badge variant="outline" className="h-5 text-xs">
              ${totalCostUsd.toFixed(4)}
            </Badge>
          </div>
          {contextTokens && (
            <div className="flex justify-between">
              <span className="text-muted-foreground">Context Tokens:</span>
              <Badge variant="outline" className="h-5 text-xs">
                {contextTokens.toLocaleString()}
              </Badge>
            </div>
          )}
        </div>
      </div>

      {workspace && (
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
      )}

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
            {!(
              gitStatus.commitsAhead ||
              gitStatus.commitsBehind ||
              gitStatus.hasUncommittedChanges ||
              gitStatus.hasConflicts
            ) && <div className="text-green-600">• Clean working directory</div>}
          </div>
        </div>
      )}

      {workspace?.environmentError && (
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

  // Get git status from workspace if available
  const gitStatus = workspace?.gitStatus
  const environmentStatus = workspace?.environmentStatus

  // Determine icon and color based on task status primarily
  let icon = statusConfig.icon
  let color = statusConfig.color
  let displayText: string = task.status

  // Override for environment issues (show these as higher priority)
  if (environmentStatus === 'error') {
    icon = <XCircle className="h-3.5 w-3.5" />
    color = 'text-red-500'
    displayText = 'env error'
  } else if (environmentStatus === 'not_created') {
    icon = <AlertCircle className="h-3.5 w-3.5" />
    color = 'text-orange-500'
    displayText = 'not setup'
  } else if (environmentStatus === 'initializing') {
    icon = <Clock className="h-3.5 w-3.5" />
    color = 'text-yellow-500'
    displayText = 'initializing'
  } else if (gitStatus && (task.status === 'open' || task.status === 'running')) {
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
  }
  // For other statuses (running, merged, closed, etc.), use the default from statusConfig

  // Calculate metrics from iterations
  const totalTurns =
    task.iterations?.reduce((sum, iteration) => sum + iteration.messageCount, 0) || 0
  const totalCostUsd = task.iterations?.reduce((sum, iteration) => sum + iteration.costUsd, 0) || 0

  // Get context tokens from the last iteration that has any context tokens
  const lastIterationWithContext = task.iterations
    ? [...task.iterations]
        .reverse()
        .find((iteration) => iteration.contextTokens && iteration.contextTokens > 0)
    : undefined
  const contextTokens = lastIterationWithContext?.contextTokens

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
        <WorkspaceStatusPopup
          totalTurns={totalTurns}
          totalCostUsd={totalCostUsd}
          contextTokens={contextTokens}
          workspace={workspace}
          gitStatus={gitStatus}
        />
      </PopoverContent>
    </Popover>
  )
}
