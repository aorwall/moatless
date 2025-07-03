import { useParams } from '@tanstack/react-router'
import { Play, Square, StopCircle, WifiOff } from 'lucide-react'
import { toast } from 'sonner'
import { useServerStatus, useStartServer, useStopServer } from '@/features/preview'
import {
  useStreamingStatus,
  useTaskData,
  useTaskStatus,
} from '@/features/tasks/contexts/task-providers'
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
import { cn } from '@/lib/utils'
import { getErrorMessage } from '@/lib/utils/error'

function StatusBadge({ status }: { status: string }) {
  const statusStyles = {
    open: 'bg-status-info/10 text-status-info border-status-info/20',
    running: 'bg-status-warning/10 text-status-warning border-status-warning/20',
    error: 'bg-status-error/10 text-status-error border-status-error/20',
    closed: 'bg-status-pending/10 text-status-pending border-status-pending/20',
    merged: 'bg-status-success/10 text-status-success border-status-success/20',
  }

  return (
    <Badge
      variant="outline"
      className={cn(
        'capitalize font-medium',
        statusStyles[status as keyof typeof statusStyles] ||
          'bg-status-pending/10 text-status-pending border-status-pending/20',
      )}
    >
      {status}
    </Badge>
  )
}

export function TaskHeader() {
  const params = useParams({ from: '/tasks/$taskId' })
  const taskId = params.taskId

  // Use specialized hooks for better performance
  const { status } = useTaskStatus()
  const { agentType, createdAt, iterations } = useTaskData()
  const { isStreaming } = useStreamingStatus()

  const { data: serverStatus, isLoading: isLoadingServerStatus } = useServerStatus(taskId, {
    enabled: !!taskId && !taskId.startsWith('temp-'),
  })

  const startServer = useStartServer({
    onSuccess: () => {
      toast.success('Server started successfully')
    },
    onError: (error) => {
      toast.error(`Failed to start server: ${getErrorMessage(error)}`)
    },
  })

  const stopServer = useStopServer({
    onSuccess: () => {
      toast.success('Server stopped successfully')
    },
    onError: (error) => {
      toast.error(`Failed to stop server: ${getErrorMessage(error)}`)
    },
  })

  const stopAgent = useStopAgent({
    onSuccess: () => {
      toast.success('Agent stopped successfully')
    },
    onError: (error) => {
      toast.error(`Failed to stop agent: ${getErrorMessage(error)}`)
    },
  })

  const handleStop = () => {
    if (status) {
      stopAgent.mutate(taskId)
    }
  }

  const handleStartServer = () => {
    if (status) {
      startServer.mutate(taskId)
    }
  }

  const handleStopServer = () => {
    if (status) {
      stopServer.mutate(taskId)
    }
  }

  if (!status) {
    return null
  }

  // Calculate total cost from iterations
  const totalCostUsd = iterations.reduce((sum, iteration) => sum + iteration.costUsd, 0)

  return (
    <header className="border-b">
      <div className="flex h-16 items-center justify-between px-4">
        <div className="flex items-center space-x-4 min-w-0 flex-1">
          <SidebarTrigger />

          <div className="space-y-1 min-w-0 flex-1 max-w-2xl">
            <div className="flex items-center space-x-2 min-w-0">
              <h1 className="text-lg font-semibold truncate">{`Task ${taskId.slice(0, 8)}`}</h1>
            </div>
            <div className="flex items-center space-x-2 text-sm text-muted-foreground overflow-hidden">
              <span className="truncate">
                {createdAt ? new Date(createdAt).toLocaleString() : ''}
              </span>
              <span className="truncate">{agentType}</span>
              <StatusBadge status={status} />
              <Badge variant="outline">{iterations.length} turns</Badge>
              <Badge variant="outline">${totalCostUsd.toFixed(4)}</Badge>
            </div>
          </div>
        </div>
        <div className="flex items-center space-x-2 flex-shrink-0">
          <div className="flex items-center space-x-2">
            {!isStreaming && status === 'running' && (
              <TooltipProvider>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <div className="flex items-center space-x-1">
                      <WifiOff className="h-4 w-4 text-status-warning" />
                      <span className="text-sm text-status-warning">Not streaming</span>
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
          {status === 'running' && (
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
