import { useParams } from '@tanstack/react-router'
import { Play, Square, StopCircle, WifiOff } from 'lucide-react'
import { toast } from 'sonner'
import { useServerStatus, useStartServer, useStopServer } from '@/features/preview'
import { useTaskData, useTaskEvents } from '@/features/tasks/contexts/task-providers'
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
  const statusColors = {
    open: 'bg-blue-500/10 text-blue-700 border-blue-500/20 dark:text-blue-400',
    running: 'bg-yellow-500/10 text-yellow-700 border-yellow-500/20 dark:text-yellow-400',
    error: 'bg-red-500/10 text-red-700 border-red-500/20 dark:text-red-400',
    closed: 'bg-gray-500/10 text-gray-700 border-gray-500/20 dark:text-gray-400',
    merged: 'bg-green-500/10 text-green-700 border-green-500/20 dark:text-green-400',
  }

  return (
    <Badge
      variant="outline"
      className={cn(
        'capitalize font-medium',
        statusColors[status as keyof typeof statusColors] ||
          'bg-gray-500/10 text-gray-700 border-gray-500/20 dark:text-gray-400',
      )}
    >
      {status}
    </Badge>
  )
}

export function TaskHeader() {
  const params = useParams({ from: '/tasks/$taskId' })
  const taskId = params.taskId

  const { status, agentType, createdAt, iterations } = useTaskData()
  const { isStreaming } = useTaskEvents()

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
