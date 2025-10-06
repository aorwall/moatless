import { X } from 'lucide-react'
import { toast } from 'sonner'
import { WorkspaceHeader } from '@/features/tasks/components/workspace-header'
import { useTaskStatus } from '@/features/tasks/contexts/task-providers'
import { useCloseTask } from '@/features/tasks/hooks/use-tasks'
import { Button } from '@/lib/components/ui/button'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/lib/components/ui/tooltip'
import { getErrorMessage } from '@/lib/utils/error'

interface FileExplorerHeaderProps {
  taskId: string
}

export function FileExplorerHeader({ taskId }: FileExplorerHeaderProps) {
  const { status, isAgentRunning } = useTaskStatus()

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
        status !== 'closed' && (
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
        )
      }
    />
  )
}
