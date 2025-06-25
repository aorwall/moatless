import type { WorkspaceStatus } from '@shared/types/workspace'
import { useParams } from '@tanstack/react-router'
import { AlertCircle, GitMerge, Loader2, Play, Square, StopCircle, WifiOff } from 'lucide-react'
import { toast } from 'sonner'
import {
  useMergeTask,
  useServerStatus,
  useStartServer,
  useStopAgent,
  useStopServer,
  useTask,
  useTaskWorkspaceStatus,
} from '@/features/tasks/hooks/use-tasks'
import { useTaskStreamingError } from '@/features/tasks/stores/task-event-store'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { SidebarTrigger } from '@/lib/components/ui/sidebar.tsx'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/lib/components/ui/tooltip'
import { getErrorMessage } from '@/lib/utils/error'

function StatusBadge({ status }: { status: string }) {
  const variant = status === 'active' ? 'default' : status === 'error' ? 'destructive' : 'secondary'
  return <Badge variant={variant}>{status}</Badge>
}

const getWorkspaceStatusDisplay = (status?: WorkspaceStatus) => {
  if (!status) {
    return { text: 'workspace: unknown', isLoading: false }
  }

  switch (status) {
    case 'not_setup':
      return { text: 'workspace: not set up', isLoading: false }
    case 'setting_up':
      return { text: 'workspace: setting up', isLoading: true }
    case 'idle':
      return { text: 'workspace: idle', isLoading: false }
    case 'running':
      return { text: 'workspace: running', isLoading: true }
    default:
      return { text: 'workspace: unknown', isLoading: false }
  }
}

export function TaskHeader() {
  const params = useParams({ from: '/tasks/$taskId' })
  const taskId = params.taskId

  const { data: metadata } = useTask(taskId)
  const {
    data: workspaceStatus,
    isLoading: isLoadingWorkspace,
    error: workspaceError,
  } = useTaskWorkspaceStatus(taskId)
  const streamingError = useTaskStreamingError(taskId)

  const { data: serverStatus, isLoading: isLoadingServerStatus } = useServerStatus(taskId, {
    enabled: !!taskId,
  })

  const startServer = useStartServer({
    onSuccess: () => {
      toast.success('Server started successfully')
    },
    onError: (error) => {
      console.error('Failed to start server:', error)
      toast.error('Failed to start server', {
        description: getErrorMessage(error),
      })
    },
  })

  const stopServer = useStopServer({
    onSuccess: () => {
      toast.success('Server stopped successfully')
    },
    onError: (error) => {
      console.error('Failed to stop server:', error)
      toast.error('Failed to stop server', {
        description: getErrorMessage(error),
      })
    },
  })

  const stopAgent = useStopAgent({
    onSuccess: () => {
      toast.success('Agent stopped successfully')
    },
    onError: (error) => {
      console.error('Failed to stop agent:', error)
      toast.error('Failed to stop agent', {
        description: getErrorMessage(error),
      })
    },
  })

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

  const handleStop = () => {
    if (metadata) {
      stopAgent.mutate(metadata.taskId)
    }
  }

  const handleStartServer = () => {
    if (metadata) {
      startServer.mutate(metadata.taskId)
    }
  }

  const handleStopServer = () => {
    if (metadata) {
      stopServer.mutate(metadata.taskId)
    }
  }

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

  const workspaceDisplay = getWorkspaceStatusDisplay(workspaceStatus?.status)

  return (
    <header className="border-b">
      <div className="flex h-16 items-center justify-between px-4">
        <div className="flex items-center space-x-4">
          <SidebarTrigger />

          <div className="space-y-1">
            <div className="flex items-center space-x-2">
              <h1 className="text-lg font-semibold">{`Task ${metadata.taskId.slice(0, 8)}`}</h1>
            </div>
            <div className="flex items-center space-x-2 text-sm text-muted-foreground">
              <span>{new Date(metadata.createdAt).toLocaleString()}</span>
              <span>{metadata.agentType}</span>
              <StatusBadge status={metadata.status} />
              <Badge variant="outline">{metadata.numTurns} turns</Badge>
              <Badge variant="outline">${metadata.totalCostUsd.toFixed(4)}</Badge>
              {workspaceError ? (
                <TooltipProvider>
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <Badge variant="destructive" className="flex items-center gap-1">
                        <AlertCircle className="h-3 w-3" />
                        workspace: error
                      </Badge>
                    </TooltipTrigger>
                    <TooltipContent>
                      <p>{getErrorMessage(workspaceError)}</p>
                    </TooltipContent>
                  </Tooltip>
                </TooltipProvider>
              ) : (
                <Badge variant="outline" className="flex items-center gap-1">
                  {(isLoadingWorkspace || workspaceDisplay.isLoading) && (
                    <Loader2 className="h-3 w-3 animate-spin" />
                  )}
                  {workspaceDisplay.text}
                </Badge>
              )}
            </div>
          </div>
        </div>
        <div className="flex items-center space-x-2">
          <div className="flex items-center space-x-2">
            {streamingError && (
              <TooltipProvider>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <div className="flex items-center space-x-1">
                      <WifiOff className="h-4 w-4 text-red-500" />
                      <span className="text-sm text-red-500">Streaming error</span>
                    </div>
                  </TooltipTrigger>
                  <TooltipContent>
                    <p>{streamingError}</p>
                  </TooltipContent>
                </Tooltip>
              </TooltipProvider>
            )}
          </div>
          {serverStatus?.configured &&
            (serverStatus.running ? (
              <TooltipProvider>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={handleStopServer}
                      disabled={stopServer.isPending || isLoadingServerStatus}
                    >
                      <Square className="mr-2 h-4 w-4" />
                      {stopServer.isPending ? 'Stopping Server...' : 'Stop Server'}
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>
                    <div className="space-y-1">
                      <p>Server is running</p>
                      <p className="text-xs text-muted-foreground">URL: {serverStatus.url}</p>
                      <p className="text-xs text-muted-foreground">PID: {serverStatus.pid}</p>
                      <p className="text-xs text-muted-foreground">
                        Uptime: {serverStatus.uptimeSeconds}s
                      </p>
                    </div>
                  </TooltipContent>
                </Tooltip>
              </TooltipProvider>
            ) : (
              <Button
                variant="outline"
                size="sm"
                onClick={handleStartServer}
                disabled={startServer.isPending || isLoadingServerStatus}
              >
                <Play className="mr-2 h-4 w-4" />
                {startServer.isPending ? 'Starting Server...' : 'Start Server'}
              </Button>
            ))}
          {metadata.status === 'active' && (
            <Button variant="outline" size="sm" onClick={handleStop} disabled={stopAgent.isPending}>
              <StopCircle className="mr-2 h-4 w-4" />
              {stopAgent.isPending ? 'Stopping...' : 'Stop'}
            </Button>
          )}
          {metadata.status !== 'active' && (
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
                      workspaceStatus?.hasUncommittedChanges
                    }
                  >
                    <GitMerge className="mr-2 h-4 w-4" />
                    {mergeTask.isPending ? 'Merging...' : 'Merge Changes'}
                  </Button>
                </TooltipTrigger>
                <TooltipContent>
                  {workspaceStatus?.hasUncommittedChanges ? (
                    <p>Cannot merge: workspace has uncommitted changes</p>
                  ) : isLoadingWorkspace ? (
                    <p>Checking workspace status...</p>
                  ) : (
                    <p>Merge workspace changes to the main branch</p>
                  )}
                </TooltipContent>
              </Tooltip>
            </TooltipProvider>
          )}
        </div>
      </div>
    </header>
  )
}
