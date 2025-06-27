import { useNavigate, useParams } from '@tanstack/react-router'
import { GitMerge, X } from 'lucide-react'
import { toast } from 'sonner'
import { useEnvironmentsList } from '@/features/environments/hooks/use-environments'
import {
  useCloseTask,
  useMergeTask,
  useTask,
  useTaskWorkspaceStatus,
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
  const params = useParams({ from: '/tasks/$taskId' })
  const navigate = useNavigate()
  const { data: metadata } = useTask(params.taskId)
  const { data: environments } = useEnvironmentsList()
  const { data: workspaceStatus, isLoading: isLoadingWorkspace } = useTaskWorkspaceStatus(
    params.taskId,
  )

  const mergeTask = useMergeTask({
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

  const closeTask = useCloseTask({
    onSuccess: () => {
      toast.success('Task closed successfully')
      // Navigate back to tasks list
      navigate({ to: '/tasks' })
    },
    onError: (error) => {
      console.error('Failed to close task:', error)
      toast.error('Failed to close task', {
        description: getErrorMessage(error),
      })
    },
  })

  const handleMerge = () => {
    if (metadata) {
      if (!workspaceStatus?.gitStatus?.hasUnmergedCommits) {
        toast.warning('No unmerged commits to merge', {
          description: 'All commits have already been merged.',
        })
        return
      }
      if (workspaceStatus?.gitStatus?.hasConflicts) {
        toast.warning('Cannot merge: conflicts detected', {
          description: 'Please resolve conflicts before merging.',
        })
        return
      }
      mergeTask.mutate({ taskId: metadata.taskId })
    }
  }

  const handleClose = () => {
    if (metadata) {
      closeTask.mutate(metadata.taskId)
    }
  }

  if (!metadata) {
    return null
  }

  // Get environment name
  const environment = environments?.find((env) => env.id === metadata.environmentId)
  const environmentName = environment?.name || metadata.environmentId

  // Get task title (first line of initial message or agent type)
  const taskTitle = metadata.iterations?.[0]?.initialMessage?.split('\n')[0] || metadata.agentType

  // Get status config
  const statusConfig = getTaskStatusConfig(metadata.status)

  return (
    <div className="h-[58px] px-4 py-2 border-b flex items-center">
      <div className="flex items-center justify-between gap-3 w-full">
        <div className="flex items-center gap-3 min-w-0">
          <SidebarTrigger />
          <div className="flex flex-col min-w-0">
            <h1 className="text-sm font-medium truncate">{taskTitle}</h1>
            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              <span className="truncate">{environmentName}</span>
              <span>•</span>
              <div className={`flex items-center gap-1 ${statusConfig.color}`}>
                {statusConfig.icon}
                <span>{metadata.status}</span>
              </div>
              {workspaceStatus?.gitStatus ? (
                <>
                  {workspaceStatus.gitStatus.hasUnmergedCommits && (
                    <>
                      <span>•</span>
                      <span className="text-blue-600 dark:text-blue-400">
                        {workspaceStatus.gitStatus.commitsAhead} ahead
                      </span>
                    </>
                  )}
                  {workspaceStatus.gitStatus.isBehindBase && (
                    <>
                      <span>•</span>
                      <span className="text-orange-600 dark:text-orange-400">
                        {workspaceStatus.gitStatus.commitsBehind} behind
                      </span>
                    </>
                  )}
                  {workspaceStatus.gitStatus.hasConflicts && (
                    <>
                      <span>•</span>
                      <span className="text-red-600 dark:text-red-400">has conflicts</span>
                    </>
                  )}
                </>
              ) : (
                <>
                  {workspaceStatus?.hasUnmergedCommits && (
                    <>
                      <span>•</span>
                      <span className="text-blue-600 dark:text-blue-400">unmerged commits</span>
                    </>
                  )}
                  {workspaceStatus?.isBehindBase && (
                    <>
                      <span>•</span>
                      <span className="text-orange-600 dark:text-orange-400">behind main</span>
                    </>
                  )}
                </>
              )}
            </div>
          </div>
        </div>
        <div className="flex items-center gap-1">
          {metadata.status !== 'running' && metadata.status !== 'closed' && (
            <TooltipProvider>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={handleMerge}
                    disabled={
                      mergeTask.isPending ||
                      isLoadingWorkspace ||
                      !workspaceStatus?.gitStatus?.hasUnmergedCommits ||
                      workspaceStatus?.gitStatus?.hasConflicts ||
                      metadata.status === 'running' ||
                      metadata.status === 'error'
                    }
                    className="h-7 px-2 cursor-pointer hover:bg-accent hover:text-accent-foreground transition-colors"
                  >
                    <GitMerge className="h-3.5 w-3.5" />
                    <span className="sr-only">Merge changes</span>
                  </Button>
                </TooltipTrigger>
                <TooltipContent>
                  {!workspaceStatus?.gitStatus?.hasUnmergedCommits ? (
                    <p>Cannot merge: workspace has uncommitted changes</p>
                  ) : metadata.status === 'running' ? (
                    <p>Cannot merge: agent is running</p>
                  ) : isLoadingWorkspace ? (
                    <p>Checking status...</p>
                  ) : !workspaceStatus?.hasUnmergedCommits ? (
                    <p>No unmerged commits to merge</p>
                  ) : workspaceStatus?.gitStatus?.hasConflicts ? (
                    <p>Cannot merge: conflicts detected</p>
                  ) : (
                    <p>Merge workspace changes to the main branch</p>
                  )}
                </TooltipContent>
              </Tooltip>
            </TooltipProvider>
          )}
          {metadata.status !== 'closed' && (
            <TooltipProvider>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={handleClose}
                    disabled={closeTask.isPending || metadata.status === 'running'}
                    className="h-7 px-2 cursor-pointer hover:bg-destructive hover:text-destructive-foreground transition-colors"
                  >
                    <X className="h-3.5 w-3.5" />
                    <span className="sr-only">Close task</span>
                  </Button>
                </TooltipTrigger>
                <TooltipContent>
                  {metadata.status === 'running' ? (
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
