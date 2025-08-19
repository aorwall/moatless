import { useParams } from '@tanstack/react-router'
import { ArrowDown, WifiOff } from 'lucide-react'
import { useEffect, useRef } from 'react'

import { useServerEvents, useServerLogs, useServerLogsStream } from '@/features/preview'
import { useScrollManager } from '@/features/tasks/hooks/use-scroll-manager'
import { Button } from '@/lib/components/ui/button'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { Skeleton } from '@/lib/components/ui/skeleton'

export function ServerLogsViewer() {
  const params = useParams({ from: '/tasks/$taskId' })
  const selectedTaskId = params.taskId
  const { serverStatus } = useServerEvents(selectedTaskId)

  // Smart streaming: use streaming for starting/started, static fetch for stopped/failed
  const shouldStream = serverStatus === 'starting' || serverStatus === 'started'
  const streamResult = useServerLogsStream(shouldStream ? selectedTaskId || '' : '')
  const staticLogsResult = useServerLogs(shouldStream ? '' : selectedTaskId || '', {
    enabled: !shouldStream,
  })

  // Use streaming logs when streaming, static logs when not streaming
  const rawLogs = shouldStream ? streamResult.logs : staticLogsResult.data
  const logs = typeof rawLogs === 'string' ? rawLogs : rawLogs?.logs || ''
  const isConnected = shouldStream ? streamResult.isConnected : false
  const error = shouldStream ? streamResult.error : staticLogsResult.error?.message || null
  const reconnect = streamResult.reconnect

  // Proper loading state detection
  const isLoading = shouldStream
    ? false // Streaming doesn't have a loading state
    : staticLogsResult.isLoading

  const scrollAreaRef = useRef<HTMLDivElement>(null)
  const bottomRef = useRef<HTMLDivElement>(null)

  // Use scroll manager for scroll detection and auto-scroll behavior
  const { isAtBottom, scrollToBottom } = useScrollManager({ scrollRef: scrollAreaRef, bottomRef })

  // Auto-scroll to bottom when new logs arrive, but only if user is at bottom
  useEffect(() => {
    if (logs && isAtBottom && scrollAreaRef.current) {
      const scrollContainer = scrollAreaRef.current.querySelector(
        '[data-radix-scroll-area-viewport]',
      )
      if (scrollContainer) {
        scrollContainer.scrollTop = scrollContainer.scrollHeight
      }
    }
  }, [logs, isAtBottom])

  if (!selectedTaskId) {
    return null
  }

  if (error && !logs) {
    return (
      <div className="flex h-full items-center justify-center p-4">
        <div className="text-center space-y-4 max-w-md">
          <ErrorAlert error={error} title="Failed to connect to server logs" />
          <Button onClick={reconnect} variant="outline">
            <WifiOff className="mr-2 h-4 w-4" />
            Retry Connection
          </Button>
        </div>
      </div>
    )
  }

  // Show skeleton only when actually loading
  if (isLoading) {
    return <ServerLogsViewerSkeleton />
  }

  if (!logs || logs.trim() === '') {
    return (
      <div className="flex h-full items-center justify-center p-8 text-center">
        <div className="text-muted-foreground space-y-4">
          <div>
            <p className="text-lg font-medium">No server logs available</p>
            <p className="mt-2 text-sm">Server logs will appear here when the server is running</p>
          </div>
          <div className="flex items-center justify-center gap-2 text-sm">
            {shouldStream && !isConnected && (
              <>
                <WifiOff className="h-4 w-4 text-muted-foreground" />
                <span>Disconnected</span>
                <Button onClick={reconnect} variant="outline" size="sm">
                  Reconnect
                </Button>
              </>
            )}
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className="h-full w-full bg-background relative">
      {/* Connection status header */}
      <div className="flex items-center justify-between px-4 py-2 border-b bg-muted/50">
        <div className="flex items-center gap-2 text-sm">
          {shouldStream && !isConnected && (
            <>
              <WifiOff className="h-3 w-3 text-muted-foreground" />
              <span className="text-muted-foreground">Disconnected</span>
            </>
          )}
        </div>
        {error && (
          <div className="flex items-center gap-2">
            <span className="text-xs text-destructive">{error}</span>
            <Button onClick={reconnect} variant="ghost" size="sm">
              Retry
            </Button>
          </div>
        )}
      </div>

      <ScrollArea ref={scrollAreaRef} className="h-full w-full">
        <div className="p-4">
          <pre className="text-xs leading-relaxed text-foreground font-mono whitespace-pre-wrap break-all">
            {logs}
          </pre>
          <div ref={bottomRef} />
        </div>
      </ScrollArea>

      {/* Scroll to bottom indicator */}
      {!isAtBottom && (
        <div className="absolute bottom-4 right-4 z-10">
          <Button
            variant="outline"
            size="sm"
            onClick={scrollToBottom}
            className="shadow-lg bg-background/80 backdrop-blur-sm border-border/50 hover:bg-background/90 flex items-center gap-2"
          >
            <ArrowDown className="h-4 w-4" />
            Scroll to bottom
          </Button>
        </div>
      )}
    </div>
  )
}

function ServerLogsViewerSkeleton() {
  const skeletonWidths = [85, 92, 87, 95, 90, 88, 93, 86]

  return (
    <div className="h-full w-full bg-background border rounded-md p-4 space-y-2">
      {skeletonWidths.map((width) => (
        <Skeleton key={`skeleton-width-${width}`} className="h-4" style={{ width: `${width}%` }} />
      ))}
    </div>
  )
}
