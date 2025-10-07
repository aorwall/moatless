import type { ServerStatusResponse } from '@moatless/types'
import { Loader2, Play, RefreshCw, Server, Square } from 'lucide-react'
import { toast } from 'sonner'
import { useRestartServer, useStartServer, useStopServer } from '@/features/preview'
import { Button } from '@/lib/components/ui/button'
import { Popover, PopoverContent, PopoverTrigger } from '@/lib/components/ui/popover'
import { cn } from '@/lib/utils'
import { getErrorMessage } from '@/lib/utils/error'
import type { ScriptHealthStatus } from '../types'
import { ScriptHealthIndicator } from './script-health-indicator'

interface ServerControlPopoverProps {
  taskId: string
  serverStatus: ServerStatusResponse | undefined
  scriptHealthStatus: ScriptHealthStatus
  isOpen: boolean
  onOpenChange: (open: boolean) => void
}

// Helper: Get server status color
function getServerStatusColor(serverStatus: ServerStatusResponse | undefined): string {
  if (!serverStatus) {
    return 'text-muted-foreground'
  }
  if (serverStatus.status === 'started') {
    return 'text-status-success'
  }
  if (serverStatus.status === 'failed') {
    return 'text-status-error'
  }
  if (serverStatus.status === 'starting') {
    return 'text-status-warning'
  }
  return 'text-muted-foreground'
}

// Helper: Get server status text
function getServerStatusText(serverStatus: ServerStatusResponse | undefined): string {
  if (!serverStatus) {
    return 'Unknown'
  }
  if (serverStatus.status === 'started') {
    return 'Running'
  }
  if (serverStatus.status === 'starting') {
    return 'Starting'
  }
  if (serverStatus.status === 'failed') {
    return 'Failed'
  }
  return 'Stopped'
}

export function ServerControlPopover({
  taskId,
  serverStatus,
  scriptHealthStatus,
  isOpen,
  onOpenChange,
}: ServerControlPopoverProps) {
  const startServer = useStartServer({
    onSuccess: (data) => {
      if (data.status === 'started') {
        toast.success('Preview environment server started successfully')
      } else if (data.status === 'failed' && data.error) {
        toast.error('Failed to start preview environment server', {
          description: data.error,
        })
      }
    },
    onError: (error) => {
      toast.error('Failed to start preview environment server', {
        description: getErrorMessage(error),
      })
    },
  })

  const stopServer = useStopServer({
    onSuccess: () => {
      toast.success('Preview environment server stopped')
    },
    onError: (error) => {
      toast.error('Failed to stop preview environment server', {
        description: getErrorMessage(error),
      })
    },
  })

  const restartServer = useRestartServer({
    onSuccess: (data) => {
      if (data.status === 'started') {
        toast.success('Preview environment server restarted successfully')
      } else if (data.status === 'failed' && data.error) {
        toast.error('Failed to restart preview environment server', {
          description: data.error,
        })
      }
    },
    onError: (error) => {
      toast.error('Failed to restart preview environment server', {
        description: getErrorMessage(error),
      })
    },
  })

  const handleStartServer = () => {
    startServer.mutate(taskId)
    onOpenChange(false)
  }

  const handleStopServer = () => {
    stopServer.mutate(taskId)
    onOpenChange(false)
  }

  const handleRestartServer = () => {
    restartServer.mutate(taskId)
    onOpenChange(false)
  }

  return (
    <Popover open={isOpen} onOpenChange={onOpenChange}>
      <PopoverTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className={cn('h-8 w-8', getServerStatusColor(serverStatus))}
          aria-label="Server"
        >
          <Server className="h-4 w-4" />
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-64 p-4" align="end">
        <div className="space-y-4">
          <div className="space-y-2">
            <h4 className="font-medium text-sm">Server Control</h4>
            <div className="flex items-center gap-2 text-sm">
              <div
                className={cn(
                  'w-2 h-2 rounded-full',
                  serverStatus?.status === 'started'
                    ? 'bg-status-success'
                    : serverStatus?.status === 'failed'
                      ? 'bg-status-error'
                      : 'bg-status-warning',
                )}
              />
              <span className="text-muted-foreground">Status:</span>
              <span className={getServerStatusColor(serverStatus)}>
                {getServerStatusText(serverStatus)}
              </span>
            </div>
            {serverStatus?.url && (
              <div className="text-sm text-muted-foreground">
                <span>URL: </span>
                <span className="font-mono">{serverStatus.url}</span>
              </div>
            )}
            {serverStatus?.uptimeSeconds !== undefined && serverStatus.status === 'started' && (
              <div className="text-sm text-muted-foreground">
                <span>Uptime: </span>
                <span>
                  {Math.floor(serverStatus.uptimeSeconds / 60)}m {serverStatus.uptimeSeconds % 60}s
                </span>
              </div>
            )}
            {serverStatus?.error && (
              <div className="text-sm text-status-error space-y-1">
                <div className="font-medium">Error:</div>
                <div className="text-xs bg-muted p-2 rounded text-foreground font-mono whitespace-pre-wrap">
                  {serverStatus.error}
                </div>
                {serverStatus.logs && (
                  <>
                    <div className="font-medium">Logs:</div>
                    <div className="text-xs bg-muted p-2 rounded text-foreground font-mono whitespace-pre-wrap max-h-32 overflow-y-auto">
                      {serverStatus.logs}
                    </div>
                  </>
                )}
              </div>
            )}
            <div className="pt-2 border-t">
              <ScriptHealthIndicator status={scriptHealthStatus} className="justify-start" />
            </div>
          </div>

          <div className="flex flex-col gap-2">
            {serverStatus?.status !== 'started' ? (
              <Button
                onClick={handleStartServer}
                disabled={startServer.isPending}
                size="sm"
                className="w-full"
              >
                {startServer.isPending ? (
                  <>
                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                    Starting...
                  </>
                ) : (
                  <>
                    <Play className="mr-2 h-4 w-4" />
                    Start Server
                  </>
                )}
              </Button>
            ) : (
              <>
                <Button
                  onClick={handleRestartServer}
                  disabled={restartServer.isPending}
                  variant="outline"
                  size="sm"
                  className="w-full"
                >
                  {restartServer.isPending ? (
                    <>
                      <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                      Restarting...
                    </>
                  ) : (
                    <>
                      <RefreshCw className="mr-2 h-4 w-4" />
                      Restart Server
                    </>
                  )}
                </Button>
                <Button
                  onClick={handleStopServer}
                  disabled={stopServer.isPending}
                  variant="destructive"
                  size="sm"
                  className="w-full"
                >
                  {stopServer.isPending ? (
                    <>
                      <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                      Stopping...
                    </>
                  ) : (
                    <>
                      <Square className="mr-2 h-4 w-4" />
                      Stop Server
                    </>
                  )}
                </Button>
              </>
            )}
          </div>
        </div>
      </PopoverContent>
    </Popover>
  )
}
