import { useParams } from '@tanstack/react-router'
import { GitMerge } from 'lucide-react'
import { toast } from 'sonner'
import { useEnvironmentsList } from '@/features/environments/hooks/use-environments'
import { useMergeTask, useTask, useTaskWorkspaceStatus } from '@/features/tasks/hooks/use-tasks'
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
  const { data: metadata } = useTask(params.taskId)
  const { data: environments } = useEnvironmentsList()
  const { data: workspaceStatus, isLoading: isLoadingWorkspace } = useTaskWorkspaceStatus(
    params.taskId,
  )

  const mergeTask = useMergeTask({
    onSuccess: (data) => {
      toast.success('Changes merged successfully', {
        description: data.message,
      })
    },
    onError: (error) => {
      console.error('Failed to merge changes:', error)
      toast.error('Failed to merge changes', {
        description: getErrorMessage(error),
      })
    },
  })

  const handleMerge = () => {
    if (metadata) {
      if (workspaceStatus?.hasUncommittedChanges) {
        toast.warning('Cannot merge: workspace has uncommitted changes', {
          description: 'Please commit or discard changes before merging.',
        })
        return
      }
      mergeTask.mutate(metadata.taskId)
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
            </div>
          </div>
        </div>
        {metadata.status !== 'running' && (
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
                    workspaceStatus?.hasUncommittedChanges ||
                    metadata.status === 'running'
                  }
                  className="h-7 px-2 cursor-pointer hover:bg-accent hover:text-accent-foreground transition-colors"
                >
                  <GitMerge className="h-3.5 w-3.5" />
                  <span className="sr-only">Merge changes</span>
                </Button>
              </TooltipTrigger>
              <TooltipContent>
                {workspaceStatus?.hasUncommittedChanges ? (
                  <p>Cannot merge: workspace has uncommitted changes</p>
                ) : metadata.status === 'running' ? (
                  <p>Cannot merge: agent is running</p>
                ) : isLoadingWorkspace ? (
                  <p>Checking status...</p>
                ) : (
                  <p>Merge workspace changes to the main branch</p>
                )}
              </TooltipContent>
            </Tooltip>
          </TooltipProvider>
        )}
      </div>
    </div>
  )
}
