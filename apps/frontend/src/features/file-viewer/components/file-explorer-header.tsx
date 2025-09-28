import { GitMerge, RefreshCw, X } from 'lucide-react'
import { toast } from 'sonner'
import { WorkspaceHeader } from '@/features/tasks/components/workspace-header'
import { useIsAgentRunning, useTaskData } from '@/features/tasks/contexts/task-data-provider'
import { useSyncWorkspace } from '@/features/tasks/hooks/use-sync-workspace'
import { useCloseTask, useMergeTask } from '@/features/tasks/hooks/use-tasks'
import { useWorkspaceStatusWithTask } from '@/features/tasks/hooks/use-workspace-status'
import { Button } from '@/lib/components/ui/button'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/lib/components/ui/tooltip'
import { getErrorMessage } from '@/lib/utils/error'

export function FileExplorerHeader() {
  const { taskId, status } = useTaskData()

  // Get workspace status and agent running state
  const workspaceQuery = useWorkspaceStatusWithTask(taskId)
  const gitStatus = workspaceQuery.data?.gitStatus
  const isAgentRunning = useIsAgentRunning()

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
    return <WorkspaceHeader />
  }

  return (
    <WorkspaceHeader
      rightContent={
        <>
          {!isAgentRunning && status !== 'closed' && status !== 'merged' && (
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
                      isAgentRunning ||
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
                  ) : isAgentRunning ? (
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
          {!isAgentRunning && status !== 'closed' && status !== 'merged' && gitStatus && (
            <TooltipProvider>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={handleSync}
                    disabled={
                      syncWorkspace.isPending ||
                      isAgentRunning ||
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
                  {isAgentRunning ? (
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
                    disabled={closeTask.isPending || isAgentRunning}
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
                  {isAgentRunning ? (
                    <p>Cannot close: agent is running</p>
                  ) : (
                    <p>Close task and teardown workspace</p>
                  )}
                </TooltipContent>
              </Tooltip>
            </TooltipProvider>
          )}
        </>
      }
    />
  )
}
