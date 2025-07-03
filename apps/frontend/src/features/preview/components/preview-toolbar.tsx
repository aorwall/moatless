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
import { type KeyboardEvent, useCallback, useEffect, useState } from 'react'
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
import { usePreviewStore } from '../stores/preview-store'

interface PreviewToolbarProps {
  onReload: () => void
}

export function PreviewToolbar({ onReload }: PreviewToolbarProps) {
  const params = useParams({ from: '/tasks/$taskId' })
  const { config, currentPath, displayPath, isInspectorActive, navigateToPath, toggleInspector } =
    usePreviewStore()

  const [pathInput, setPathInput] = useState(displayPath)
  const [isServerPopoverOpen, setIsServerPopoverOpen] = useState(false)

  // Sync pathInput with displayPath when it changes from route navigation
  useEffect(() => {
    setPathInput(displayPath)
  }, [displayPath])

  // Server management hooks
  const { data: serverStatus } = useServerStatus(params.taskId)
  const startServer = useStartServer()
  const stopServer = useStopServer()
  const restartServer = useRestartServer()

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
    return 'text-status-error'
  }

  const getServerStatusText = () => {
    if (!serverStatus) return 'Unknown'
    if (serverStatus.running) {
      return 'Running'
    }
    return 'Stopped'
  }

  return (
    <div className="flex items-center gap-2 p-2 border-b bg-muted/30">
      {/* Navigation Controls */}
      <div className="flex items-center gap-1">
        <Tooltip>
          <TooltipTrigger asChild>
            <Button variant="ghost" size="icon" onClick={onReload} className="h-8 w-8">
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
        />
      </div>

      {/* Tool Controls */}
      <div className="flex items-center gap-1">
        {/* Server Control Popup */}
        <Popover open={isServerPopoverOpen} onOpenChange={setIsServerPopoverOpen}>
          <PopoverTrigger asChild>
            <Button variant="ghost" size="icon" className={cn('h-8 w-8', getServerStatusColor())}>
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
