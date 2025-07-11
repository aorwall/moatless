import { useParams } from '@tanstack/react-router'
import { useEffect, useRef } from 'react'

import { useServerLogs } from '@/features/preview'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { Skeleton } from '@/lib/components/ui/skeleton'

export function ServerLogsViewer() {
  const params = useParams({ from: '/tasks/$taskId' })
  const selectedTaskId = params.taskId
  const { data: response, isLoading, error } = useServerLogs(selectedTaskId || '')
  const scrollAreaRef = useRef<HTMLDivElement>(null)
  const shouldAutoScroll = useRef(true)

  // Auto-scroll to bottom when new logs arrive
  useEffect(() => {
    if (response?.logs && shouldAutoScroll.current && scrollAreaRef.current) {
      const scrollContainer = scrollAreaRef.current.querySelector(
        '[data-radix-scroll-area-viewport]',
      )
      if (scrollContainer) {
        scrollContainer.scrollTop = scrollContainer.scrollHeight
      }
    }
  }, [response?.logs])

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

  const handleScroll = (e: React.UIEvent<HTMLDivElement>) => {
    const element = e.currentTarget
    const isAtBottom =
      Math.abs(element.scrollHeight - element.scrollTop - element.clientHeight) < 10
    shouldAutoScroll.current = isAtBottom
  }

  return (
    <div className="h-full w-full bg-background border rounded-md">
      <ScrollArea ref={scrollAreaRef} className="h-full w-full" onScroll={handleScroll}>
        <div className="p-4">
          <pre className="text-xs leading-relaxed text-foreground font-mono whitespace-pre-wrap break-all">
            {logContent}
          </pre>
        </div>
      </ScrollArea>
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
