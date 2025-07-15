import type { Task } from '@moatless/types'
import { GitMerge, RefreshCw, X } from 'lucide-react'
import { toast } from 'sonner'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { TaskStatusIcon } from '@/features/tasks/components/task-status-icon'
import { useTaskData } from '@/features/tasks/contexts/task-data-provider'
import { usePanelView } from '@/features/tasks/contexts/task-ui-provider'
import { useSyncWorkspace } from '@/features/tasks/hooks/use-sync-workspace'
import { useCloseTask, useMergeTask } from '@/features/tasks/hooks/use-tasks'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { SidebarTrigger } from '@/lib/components/ui/sidebar'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/lib/components/ui/tooltip'
import { cn } from '@/lib/utils'
import { getErrorMessage } from '@/lib/utils/error'

export function TaskHeader() {
  const { taskId, status, agentType, repositoryId, iterations, workspaceGitStatus } = useTaskData()
  const { data: repositories } = useRepositoriesList()
  const { rightPanelView, setRightPanelView } = usePanelView()

  // Use workspace git status instead of iteration git sync status
  const gitStatus = workspaceGitStatus

  const mergeTask = useMergeTask(taskId, {
    onSuccess: () => {
      toast.success('Changes merged successfully')
    },
    onError: (error) => {
      console.error('Failed to merge changes:', error)
      toast.error('Failed to merge changes', {
        description: getErrorMessage(error),
      })
    },
  })

  const closeTask = useCloseTask(taskId, {
    onSuccess: () => {
      toast.success('Task closed successfully')
    },
    onError: (error) => {
      console.error('Failed to close task:', error)
      toast.error('Failed to close task', {
        description: getErrorMessage(error),
      })
    },
  })

  const syncWorkspace = useSyncWorkspace({
    onSuccess: (data) => {
      if (data.success) {
        toast.success('Workspace synced successfully', {
          description: data.message,
        })
      } else {
        toast.warning('Could not sync workspace', {
          description: data.message,
        })
      }
    },
    onError: (error) => {
      console.error('Failed to sync workspace:', error)
      toast.error('Failed to sync workspace', {
        description: getErrorMessage(error),
      })
    },
  })

  const handleMerge = () => {
    if (taskId && status) {
      if (!gitStatus?.commitsAhead || gitStatus.commitsAhead === 0) {
        toast.warning('No unmerged commits to merge', {
          description: 'All commits have already been merged.',
        })
        return
      }
      if (gitStatus?.status === 'diverged' || gitStatus?.hasConflicts) {
        toast.warning('Cannot merge: conflicts detected', {
          description: 'Please resolve conflicts before merging.',
        })
        return
      }
      mergeTask.mutate({})
    }
  }

  const handleSync = () => {
    if (taskId) {
      syncWorkspace.mutate({ taskId })
    }
  }

  const handleClose = () => {
    if (taskId) {
      closeTask.mutate()
    }
  }

  if (!status) {
    return null
  }

  // Get repository name
  const repository = repositories?.find((repo) => repo.id === repositoryId)
  const repositoryName = repository?.name || repositoryId

  // Get task title (first line of initial message or agent type)
  const taskTitle = iterations?.[0]?.initialMessage?.split('\n')[0] || agentType

  // Create task object for TaskStatusIcon
  const task = { status, iterations } as Pick<Task, 'status' | 'iterations'>

  // Format git status info in compact format
  const gitStatusInfo = gitStatus
    ? [
        gitStatus.commitsAhead > 0 && `${gitStatus.commitsAhead} ahead`,
        gitStatus.commitsBehind > 0 && `${gitStatus.commitsBehind} behind`,
        (gitStatus.status === 'diverged' || gitStatus.hasConflicts) && 'has conflicts',
        gitStatus.hasUncommittedChanges && 'uncommitted changes',
      ]
        .filter(Boolean)
        .join(', ')
    : ''

  // Calculate metrics from iterations
  const totalTurns = iterations?.reduce((sum, iteration) => sum + iteration.messageCount, 0) || 0
  const totalCostUsd = iterations?.reduce((sum, iteration) => sum + iteration.costUsd, 0) || 0

  // Get context tokens from the last iteration that has any context tokens
  const lastIterationWithContext = iterations
    ? [...iterations]
        .reverse()
        .find((iteration) => iteration.contextTokens && iteration.contextTokens > 0)
    : undefined
  const contextTokens = lastIterationWithContext?.contextTokens
  return (
    <div className="h-[58px] px-4 py-2 border-b flex items-center">
      <div className="flex items-center justify-between gap-3 w-full">
        {/* Left side */}
        <div className="flex items-center gap-3 min-w-0">
          <SidebarTrigger />
          <div className="flex items-center gap-2 text-sm">
            <span className="font-medium truncate max-w-[300px]" title={taskTitle}>
              {taskTitle}
            </span>
            <span className="text-muted-foreground">•</span>
            <span className="text-muted-foreground truncate max-w-[150px]" title={repositoryName}>
              {repositoryName}
            </span>
            <span className="text-muted-foreground">•</span>
            <TaskStatusIcon task={task} />
            {gitStatusInfo && (
              <>
                <span className="text-muted-foreground">•</span>
                <span className="text-xs text-muted-foreground">{gitStatusInfo}</span>
              </>
            )}
            <span className="text-muted-foreground">•</span>
            <Badge variant="outline" className="text-xs">
              {totalTurns} turns
            </Badge>
            <Badge variant="outline" className="text-xs">
              ${totalCostUsd.toFixed(4)}
            </Badge>
            {contextTokens && (
              <Badge variant="outline" className="text-xs">
                {contextTokens.toLocaleString()} tokens
              </Badge>
            )}
          </div>
        </div>

        {/* Right side */}
        <div className="flex items-center gap-2">
          {/* View toggle */}
          <div className="flex items-center border rounded-md bg-muted/30">
            <button
              type="button"
              className={cn(
                'px-3 py-1 text-sm font-medium transition-colors rounded-l-md',
                rightPanelView === 'files'
                  ? 'bg-background text-foreground shadow-sm'
                  : 'text-muted-foreground hover:text-foreground',
              )}
              onClick={() => setRightPanelView('files')}
            >
              Files
            </button>
            <button
              type="button"
              className={cn(
                'px-3 py-1 text-sm font-medium transition-colors rounded-r-md',
                rightPanelView === 'preview'
                  ? 'bg-background text-foreground shadow-sm'
                  : 'text-muted-foreground hover:text-foreground',
              )}
              onClick={() => setRightPanelView('preview')}
            >
              Preview
            </button>
          </div>

          {/* Action buttons */}
          {status !== 'running' && status !== 'closed' && status !== 'merged' && (
            <TooltipProvider>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={handleMerge}
                    disabled={
                      mergeTask.isPending ||
                      !gitStatus?.commitsAhead ||
                      gitStatus.commitsAhead === 0 ||
                      gitStatus.status === 'diverged' ||
                      gitStatus.hasConflicts ||
                      status === 'running' ||
                      status === 'error'
                    }
                    className="h-7 px-2"
                  >
                    <GitMerge className="h-3.5 w-3.5" />
                    <span className="sr-only">Merge changes</span>
                  </Button>
                </TooltipTrigger>
                <TooltipContent>
                  {!gitStatus?.commitsAhead || gitStatus.commitsAhead === 0 ? (
                    <p>No unmerged commits to merge</p>
                  ) : status === 'running' ? (
                    <p>Cannot merge: agent is running</p>
                  ) : gitStatus.status === 'diverged' || gitStatus.hasConflicts ? (
                    <p>Cannot merge: conflicts detected</p>
                  ) : (
                    <p>Merge workspace changes to the main branch</p>
                  )}
                </TooltipContent>
              </Tooltip>
            </TooltipProvider>
          )}
          {status !== 'running' && status !== 'closed' && status !== 'merged' && gitStatus && (
            <TooltipProvider>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={handleSync}
                    disabled={
                      syncWorkspace.isPending ||
                      status === 'running' ||
                      status === 'error' ||
                      !gitStatus.commitsBehind ||
                      gitStatus.commitsBehind === 0 ||
                      gitStatus.hasConflicts ||
                      gitStatus.hasUncommittedChanges ||
                      gitStatus.status === 'diverged'
                    }
                    className="h-7 px-2"
                  >
                    {syncWorkspace.isPending ? (
                      <div className="h-3.5 w-3.5 animate-spin rounded-full border-2 border-current border-t-transparent" />
                    ) : (
                      <RefreshCw className="h-3.5 w-3.5" />
                    )}
                    <span className="sr-only">Sync with main</span>
                  </Button>
                </TooltipTrigger>
                <TooltipContent>
                  {status === 'running' ? (
                    <p>Cannot sync: agent is running</p>
                  ) : !gitStatus.commitsBehind || gitStatus.commitsBehind === 0 ? (
                    <p>Already up to date with main</p>
                  ) : gitStatus.hasConflicts || gitStatus.status === 'diverged' ? (
                    <p>Cannot sync: conflicts detected</p>
                  ) : gitStatus.hasUncommittedChanges ? (
                    <p>Cannot sync: uncommitted changes detected</p>
                  ) : (
                    <p>
                      Sync workspace with main branch ({gitStatus.commitsBehind} commits behind)
                    </p>
                  )}
                </TooltipContent>
              </Tooltip>
            </TooltipProvider>
          )}
          {status !== 'closed' && (
            <TooltipProvider>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={handleClose}
                    disabled={closeTask.isPending || status === 'running'}
                    className="h-7 px-2"
                  >
                    {closeTask.isPending ? (
                      <div className="h-3.5 w-3.5 animate-spin rounded-full border-2 border-current border-t-transparent" />
                    ) : (
                      <X className="h-3.5 w-3.5" />
                    )}
                    <span className="sr-only">Close task</span>
                  </Button>
                </TooltipTrigger>
                <TooltipContent>
                  {status === 'running' ? (
                    <p>Cannot close: agent is running</p>
                  ) : (
                    <p>Close task and teardown workspace</p>
                  )}
                </TooltipContent>
              </Tooltip>
            </TooltipProvider>
          )}
        </div>
      </div>
    </div>
  )
}
