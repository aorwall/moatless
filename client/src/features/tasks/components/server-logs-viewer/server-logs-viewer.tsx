import { useEffect, useRef } from 'react'

import { useServerLogs } from '@/features/tasks/hooks/use-tasks'
import { useTaskUIStore } from '@/features/tasks/stores/task-ui-store'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { Skeleton } from '@/lib/components/ui/skeleton'

export function ServerLogsViewer() {
  const selectedTaskId = useTaskUIStore((state) => state.selectedTaskId)
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
    <div className="h-full w-full bg-slate-950 dark:bg-black">
      <ScrollArea ref={scrollAreaRef} className="h-full w-full" onScroll={handleScroll}>
        <div className="p-4">
          <pre className="text-xs leading-relaxed text-slate-100 dark:text-slate-200 font-mono whitespace-pre-wrap break-all">
            {logContent}
          </pre>
        </div>
      </ScrollArea>
    </div>
  )
}

function ServerLogsViewerSkeleton() {
  return (
    <div className="h-full w-full bg-slate-950 dark:bg-black p-4 space-y-2">
      {Array.from({ length: 8 }).map((_, i) => (
        <Skeleton
          key={i}
          className="h-4 bg-slate-800 dark:bg-slate-900"
          style={{ width: `${85 + Math.random() * 15}%` }}
        />
      ))}
    </div>
  )
}
