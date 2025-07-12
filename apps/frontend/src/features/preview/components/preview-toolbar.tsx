import { useParams } from '@tanstack/react-router'
import {
  ExternalLink,
  Loader2,
  MousePointer,
  Play,
  RefreshCw,
  RotateCcw,
  Server,
  Square,
} from 'lucide-react'
import { type KeyboardEvent, useCallback, useState } from 'react'
import { toast } from 'sonner'
import {
  useRestartServer,
  useServerStatus,
  useStartServer,
  useStopServer,
} from '@/features/preview'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'
import { Popover, PopoverContent, PopoverTrigger } from '@/lib/components/ui/popover'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/lib/components/ui/tooltip'
import { cn } from '@/lib/utils'
import { usePreview } from '../hooks/use-preview'
import { ScriptHealthIndicator } from './script-health-indicator'

interface PreviewToolbarProps {
  onReload: () => void
}

export function PreviewToolbar({ onReload }: PreviewToolbarProps) {
  const params = useParams({ from: '/tasks/$taskId' })
  const {
    config,
    currentPath,
    displayPath,
    isInspectorActive,
    scriptHealthStatus,
    navigateToPath,
    toggleInspector,
  } = usePreview()

  const [pathInput, setPathInput] = useState(displayPath)
  const [isServerPopoverOpen, setIsServerPopoverOpen] = useState(false)

  // Reset pathInput when displayPath changes (using key prop pattern in parent is better)
  // But for internal component state, we need to track when displayPath changes
  const [lastDisplayPath, setLastDisplayPath] = useState(displayPath)

  if (displayPath !== lastDisplayPath) {
    setPathInput(displayPath)
    setLastDisplayPath(displayPath)
  }

  // Server management hooks
  const { data: serverStatus } = useServerStatus(params.taskId)
  const startServer = useStartServer({
    onSuccess: (data) => {
      if (data.running) {
        toast.success('Preview environment server started successfully')
      } else if (data.error) {
        toast.error('Failed to start preview environment server', {
          description: data.error,
        })
      }
    },
    onError: (error) => {
      toast.error('Failed to start preview environment server', {
        description: error.message,
      })
    },
  })
  const stopServer = useStopServer({
    onSuccess: () => {
      toast.success('Preview environment server stopped')
    },
    onError: (error) => {
      toast.error('Failed to stop preview environment server', {
        description: error.message,
      })
    },
  })
  const restartServer = useRestartServer({
    onSuccess: (data) => {
      if (data.running) {
        toast.success('Preview environment server restarted successfully')
      } else if (data.error) {
        toast.error('Failed to restart preview environment server', {
          description: data.error,
        })
      }
    },
    onError: (error) => {
      toast.error('Failed to restart preview environment server', {
        description: error.message,
      })
    },
  })

  const handlePathSubmit = useCallback(
    (e: KeyboardEvent<HTMLInputElement>) => {
      if (e.key === 'Enter') {
        let path = pathInput.trim()
        if (!path.startsWith('/')) {
          path = `/${path}`
        }
        navigateToPath(path)
      }
    },
    [pathInput, navigateToPath],
  )

  const handleOpenInNewTab = useCallback(() => {
    const fullUrl = `${config.url}${currentPath}`
    window.open(fullUrl, '_blank', 'noopener,noreferrer')
  }, [config.url, currentPath])

  const handleStartServer = useCallback(() => {
    startServer.mutate(params.taskId)
    setIsServerPopoverOpen(false)
  }, [params.taskId, startServer])

  const handleStopServer = useCallback(() => {
    stopServer.mutate(params.taskId)
    setIsServerPopoverOpen(false)
  }, [params.taskId, stopServer])

  const handleRestartServer = useCallback(() => {
    restartServer.mutate(params.taskId)
    setIsServerPopoverOpen(false)
  }, [params.taskId, restartServer])

  const getServerStatusColor = () => {
    if (!serverStatus) return 'text-muted-foreground'
    if (serverStatus.running) return 'text-status-success'
    if (serverStatus.error) return 'text-status-error'
    return 'text-muted-foreground'
  }

  const getServerStatusText = () => {
    if (!serverStatus) return 'Unknown'
    if (serverStatus.running) {
      return 'Running'
    }
    if (serverStatus.error) {
      return 'Failed'
    }
    return 'Stopped'
  }

  return (
    <div className="flex items-center gap-2 p-2 border-b bg-muted/30">
      {/* Navigation Controls */}
      <div className="flex items-center gap-1">
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              onClick={onReload}
              className="h-8 w-8"
              aria-label="Reload"
            >
              <RotateCcw className="h-4 w-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Reload Preview</TooltipContent>
        </Tooltip>
      </div>

      {/* Address Bar */}
      <div className="flex-1 flex items-center gap-2">
        <Input
          type="text"
          value={pathInput}
          onChange={(e) => setPathInput(e.target.value)}
          onKeyDown={handlePathSubmit}
          placeholder="Enter path (e.g., /about)"
          className="flex-1 h-8 text-sm"
          aria-label="Path"
        />
      </div>

      {/* Tool Controls */}
      <div className="flex items-center gap-1">
        {/* Server Control Popup */}
        <Popover open={isServerPopoverOpen} onOpenChange={setIsServerPopoverOpen}>
          <PopoverTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className={cn('h-8 w-8', getServerStatusColor())}
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
                      serverStatus?.running ? 'bg-status-success' : 'bg-status-error',
                    )}
                  />
                  <span className="text-muted-foreground">Status:</span>
                  <span className={getServerStatusColor()}>{getServerStatusText()}</span>
                </div>
                {serverStatus?.url && (
                  <div className="text-sm text-muted-foreground">
                    <span>URL: </span>
                    <span className="font-mono">{serverStatus.url}</span>
                  </div>
                )}
                {serverStatus?.uptimeSeconds !== undefined && serverStatus.running && (
                  <div className="text-sm text-muted-foreground">
                    <span>Uptime: </span>
                    <span>
                      {Math.floor(serverStatus.uptimeSeconds / 60)}m{' '}
                      {serverStatus.uptimeSeconds % 60}s
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
                {!serverStatus?.running ? (
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

        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant={isInspectorActive ? 'default' : 'ghost'}
              size="icon"
              onClick={toggleInspector}
              className="h-8 w-8"
              aria-label="Toggle Inspector"
            >
              <MousePointer className="h-4 w-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Toggle Element Inspector</TooltipContent>
        </Tooltip>

        <Tooltip>
          <TooltipTrigger asChild>
            <Button variant="ghost" size="icon" onClick={handleOpenInNewTab} className="h-8 w-8">
              <ExternalLink className="h-4 w-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Open in New Tab</TooltipContent>
        </Tooltip>
      </div>
    </div>
  )
}
