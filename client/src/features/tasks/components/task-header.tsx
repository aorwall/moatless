import { useParams } from '@tanstack/react-router'
import { Play, Square, StopCircle, WifiOff } from 'lucide-react'
import { toast } from 'sonner'
import { useServerStatus, useStartServer, useStopServer } from '@/features/preview'
import { useTaskDataContext } from '@/features/tasks/contexts/task-data-context'
import { useStopAgent } from '@/features/tasks/hooks/use-tasks'
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
  let variant: 'default' | 'destructive' | 'secondary' | 'outline' = 'secondary'
  const displayStatus = status

  switch (status) {
    case 'open':
      variant = 'default'
      break
    case 'running':
      variant = 'outline'
      break
    case 'error':
      variant = 'destructive'
      break
    case 'merged':
      variant = 'secondary'
      break
    case 'closed':
      variant = 'secondary'
      break
  }

  return <Badge variant={variant}>{displayStatus}</Badge>
}

export function TaskHeader() {
  const params = useParams({ from: '/tasks/$taskId' })
  const taskId = params.taskId

  const { task: metadata, isStreaming } = useTaskDataContext()

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

  if (!metadata) {
    return null
  }

  return (
    <header className="border-b">
      <div className="flex h-16 items-center justify-between px-4">
        <div className="flex items-center space-x-4 min-w-0 flex-1">
          <SidebarTrigger />

          <div className="space-y-1 min-w-0 flex-1 max-w-2xl">
            <div className="flex items-center space-x-2 min-w-0">
              <h1 className="text-lg font-semibold truncate">{`Task ${metadata.taskId.slice(0, 8)}`}</h1>
            </div>
            <div className="flex items-center space-x-2 text-sm text-muted-foreground overflow-hidden">
              <span className="truncate">{new Date(metadata.createdAt).toLocaleString()}</span>
              <span className="truncate">{metadata.agentType}</span>
              <StatusBadge status={metadata.status} />
              <Badge variant="outline">{metadata.numTurns} turns</Badge>
              <Badge variant="outline">${metadata.totalCostUsd.toFixed(4)}</Badge>
            </div>
          </div>
        </div>
        <div className="flex items-center space-x-2 flex-shrink-0">
          <div className="flex items-center space-x-2">
            {!isStreaming && metadata?.status === 'running' && (
              <TooltipProvider>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <div className="flex items-center space-x-1">
                      <WifiOff className="h-4 w-4 text-yellow-500" />
                      <span className="text-sm text-yellow-500">Not streaming</span>
                    </div>
                  </TooltipTrigger>
                  <TooltipContent>
                    <p>Real-time streaming is not connected</p>
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
          {metadata.status === 'running' && (
            <Button variant="outline" size="sm" onClick={handleStop} disabled={stopAgent.isPending}>
              <StopCircle className="mr-2 h-4 w-4" />
              {stopAgent.isPending ? 'Stopping...' : 'Stop Agent'}
            </Button>
          )}
        </div>
      </div>
    </header>
  )
}
