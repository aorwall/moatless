import { GitMerge, X } from 'lucide-react'
import { toast } from 'sonner'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { useTaskData } from '@/features/tasks/contexts/task-data-provider'
import {
  useCloseTask,
  useMergeTask,
} from '@/features/tasks/hooks/use-tasks'
import { getTaskStatusConfig } from '@/features/tasks/utils/task-status'
import { Button } from '@/lib/components/ui/button'
import { SidebarTrigger } from '@/lib/components/ui/sidebar'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/lib/components/ui/tooltip'
import { getErrorMessage } from '@/lib/utils/error'

export function ChatHeader() {
  const { taskId, status, agentType, repositoryId, iterations } = useTaskData()
  const { data: repositories } = useRepositoriesList()
  
  // Get git sync status from the last iteration
  const lastIteration = iterations?.[iterations.length - 1]
  const gitSyncStatus = lastIteration?.gitSyncStatus

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

  const handleMerge = () => {
    if (taskId && status) {
      if (!gitSyncStatus?.commitsAhead || gitSyncStatus.commitsAhead === 0) {
        toast.warning('No unmerged commits to merge', {
          description: 'All commits have already been merged.',
        })
        return
      }
      if (gitSyncStatus?.status === 'diverged') {
        toast.warning('Cannot merge: conflicts detected', {
          description: 'Please resolve conflicts before merging.',
        })
        return
      }
      mergeTask.mutate({})
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

  // Get status config
  const statusConfig = getTaskStatusConfig(status)

  return (
    <div className="h-[58px] px-4 py-2 border-b flex items-center">
      <div className="flex items-center justify-between gap-3 w-full">
        <div className="flex items-center gap-3 min-w-0">
          <SidebarTrigger />
          <div className="flex flex-col min-w-0">
            <h1 className="text-sm font-medium truncate">{taskTitle}</h1>
            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              <span className="truncate">{repositoryName}</span>
              <span>•</span>
              <div className={`flex items-center gap-1 ${statusConfig.color}`}>
                {statusConfig.icon}
                <span>{status}</span>
              </div>
              {gitSyncStatus && (
                <>
                  {gitSyncStatus.commitsAhead > 0 && (
                    <>
                      <span>•</span>
                      <span className="text-status-info">
                        {gitSyncStatus.commitsAhead} ahead
                      </span>
                    </>
                  )}
                  {gitSyncStatus.commitsBehind > 0 && (
                    <>
                      <span>•</span>
                      <span className="text-status-warning">
                        {gitSyncStatus.commitsBehind} behind
                      </span>
                    </>
                  )}
                  {gitSyncStatus.status === 'diverged' && (
                    <>
                      <span>•</span>
                      <span className="text-status-error">has conflicts</span>
                    </>
                  )}
                  {gitSyncStatus.hasUncommittedChanges && (
                    <>
                      <span>•</span>
                      <span className="text-status-warning">
                        uncommitted changes
                      </span>
                    </>
                  )}
                </>
              )}
            </div>
          </div>
        </div>
        <div className="flex items-center gap-1">
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
                      !gitSyncStatus?.commitsAhead ||
                      gitSyncStatus.commitsAhead === 0 ||
                      gitSyncStatus.status === 'diverged' ||
                      status === 'running' ||
                      status === 'error'
                    }
                    className="h-7 px-2 cursor-pointer hover:bg-accent hover:text-accent-foreground transition-colors"
                  >
                    <GitMerge className="h-3.5 w-3.5" />
                    <span className="sr-only">Merge changes</span>
                  </Button>
                </TooltipTrigger>
                <TooltipContent>
                  {!gitSyncStatus?.commitsAhead || gitSyncStatus.commitsAhead === 0 ? (
                    <p>No unmerged commits to merge</p>
                  ) : status === 'running' ? (
                    <p>Cannot merge: agent is running</p>
                  ) : gitSyncStatus.status === 'diverged' ? (
                    <p>Cannot merge: conflicts detected</p>
                  ) : (
                    <p>Merge workspace changes to the main branch</p>
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
                    className="h-7 px-2 cursor-pointer hover:bg-destructive hover:text-destructive-foreground transition-colors"
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
