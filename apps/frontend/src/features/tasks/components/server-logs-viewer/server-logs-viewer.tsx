import { useParams } from '@tanstack/react-router'
import { ArrowDown, RefreshCw } from 'lucide-react'
import { useRef } from 'react'

import { useServerEvents, useServerLogs } from '@/features/preview'
import { useAutoScroll } from '@/features/tasks/hooks/use-auto-scroll'
import { Button } from '@/lib/components/ui/button'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { Skeleton } from '@/lib/components/ui/skeleton'

export function ServerLogsViewer() {
  const params = useParams({ from: '/tasks/$taskId' })
  const selectedTaskId = params.taskId
  const { serverStatus } = useServerEvents(selectedTaskId)

  // Poll logs when server is running (starting or started)
  const shouldPoll = serverStatus === 'starting' || serverStatus === 'started'
  const {
    data: logsData,
    error: logsError,
    isLoading,
    refetch,
  } = useServerLogs(selectedTaskId || '', {
    enabled: !!selectedTaskId,
    refetchInterval: shouldPoll ? 1000 : false, // Poll every 1 second when server is running
  })

  const logs = logsData?.logs || ''
  const error = logsError?.message || null

  const bottomRef = useRef<HTMLDivElement>(null)

  // Use auto-scroll for scroll detection and auto-scroll behavior
  const {
    scrollRef: scrollAreaRef,
    isAtBottom,
    scrollToBottom,
  } = useAutoScroll({
    scrollOnMount: true,
    dependencies: [logs], // Re-evaluate scroll when logs change
  })

  if (!selectedTaskId) {
    return null
  }

  if (error && !logs) {
    return (
      <div className="flex h-full items-center justify-center p-4">
        <div className="text-center space-y-4 max-w-md">
          <ErrorAlert error={error} title="Failed to load server logs" />
          <Button onClick={() => refetch()} variant="outline">
            <RefreshCw className="mr-2 h-4 w-4" />
            Retry
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
        </div>
      </div>
    )
  }

  return (
    <div className="h-full w-full bg-background relative">
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
