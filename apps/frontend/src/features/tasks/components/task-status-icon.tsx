import type { TaskResponse, WorkspaceStatusFullResponse } from '@moatless/api-client'
import { AlertCircle, CheckCircle, Clock, XCircle } from 'lucide-react'
import { getTaskStatusConfig } from '@/features/tasks/utils/task-status'
import { Badge } from '@/lib/components/ui/badge'
import { Popover, PopoverContent, PopoverTrigger } from '@/lib/components/ui/popover'

interface WorkspaceStatusPopupProps {
  totalCostUsd: number
  workspace?: WorkspaceStatusFullResponse
}

function WorkspaceStatusPopup({ totalCostUsd, workspace }: WorkspaceStatusPopupProps) {
  return (
    <div className="space-y-3 w-64">
      <div>
        <h4 className="font-medium text-sm mb-2">Task Statistics</h4>
        <div className="space-y-2 text-xs">
          <div className="flex justify-between">
            <span className="text-muted-foreground">Total Cost:</span>
            <Badge variant="outline" className="h-5 text-xs">
              ${totalCostUsd.toFixed(4)}
            </Badge>
          </div>
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
  task: Pick<TaskResponse, 'status' | 'totalCostUsd'>
  workspace?: WorkspaceStatusFullResponse
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
  } else if (task.status === 'open' && environmentStatus === 'ready') {
    // Task is open and environment is ready
    icon = <CheckCircle className="h-3.5 w-3.5" />
    color = 'text-green-500'
    displayText = 'ready'
  }
  // For other statuses (running, merged, closed, etc.), use the default from statusConfig

  const totalCostUsd = task.totalCostUsd || 0

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
        <WorkspaceStatusPopup totalCostUsd={totalCostUsd} workspace={workspace} />
      </PopoverContent>
    </Popover>
  )
}
