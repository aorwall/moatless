import { useParams } from '@tanstack/react-router'
import { ArrowDown } from 'lucide-react'
import { useEffect, useRef } from 'react'

import { useServerLogs } from '@/features/preview'
import { useScrollManager } from '@/features/tasks/hooks/use-scroll-manager'
import { Button } from '@/lib/components/ui/button'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { Skeleton } from '@/lib/components/ui/skeleton'

export function ServerLogsViewer() {
  const params = useParams({ from: '/tasks/$taskId' })
  const selectedTaskId = params.taskId
  const { data: response, isLoading, error } = useServerLogs(selectedTaskId || '')
  const scrollAreaRef = useRef<HTMLDivElement>(null)
  const bottomRef = useRef<HTMLDivElement>(null)

  // Use scroll manager for scroll detection and auto-scroll behavior
  const { isAtBottom, scrollToBottom } = useScrollManager({ scrollRef: scrollAreaRef, bottomRef })

  // Auto-scroll to bottom when new logs arrive, but only if user is at bottom
  useEffect(() => {
    if (response?.logs && isAtBottom && scrollAreaRef.current) {
      const scrollContainer = scrollAreaRef.current.querySelector(
        '[data-radix-scroll-area-viewport]',
      )
      if (scrollContainer) {
        scrollContainer.scrollTop = scrollContainer.scrollHeight
      }
    }
  }, [response?.logs, isAtBottom])

  if (!selectedTaskId) {
    return null
  }

  if (error) {
    return (
      <div className="flex h-full items-center justify-center p-4">
        <ErrorAlert error={error} title="Failed to load server logs" className="max-w-md" />
      </div>
    )
  }

  if (isLoading) {
    return <ServerLogsViewerSkeleton />
  }

  const logContent = response?.logs || ''

  if (!logContent || logContent.trim() === '') {
    return (
      <div className="flex h-full items-center justify-center p-8 text-center">
        <div className="text-muted-foreground">
          <p className="text-lg font-medium">No server logs available</p>
          <p className="mt-2 text-sm">Server logs will appear here when the server is running</p>
        </div>
      </div>
    )
  }

  return (
    <div className="h-full w-full bg-background border rounded-md relative">
      <ScrollArea ref={scrollAreaRef} className="h-full w-full">
        <div className="p-4">
          <pre className="text-xs leading-relaxed text-foreground font-mono whitespace-pre-wrap break-all">
            {logContent}
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
