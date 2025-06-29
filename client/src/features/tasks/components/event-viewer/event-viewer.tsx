import { useParams } from '@tanstack/react-router'
import { FileText, Loader2 } from 'lucide-react'
import { useMemo } from 'react'
import { useTaskData, useTaskEvents } from '@/features/tasks/contexts/task-providers'
import { useAutoScroll } from '@/features/tasks/hooks/use-auto-scroll'
import { mergeToolEvents } from '@/features/tasks/utils/merge-tool-events'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { ScrollArea } from '@/lib/components/ui/scroll-area'

import { EventList } from './event-list'

// Track visited tasks to determine if this is the first time opening this task
const _visitedTasks = new Set<string>()

export function EventViewer() {
  const _params = useParams({ from: '/tasks/$taskId' })
  const { scrollRef } = useAutoScroll()

  // Use the context to get events and states
  const { events, isStreaming } = useTaskEvents()
  const { isLoading, error } = useTaskData()

  // Merge tool events within each event for cleaner display
  const mergedEvents = useMemo(() => {
    return mergeToolEvents(events)
  }, [events])

  if (error) {
    return (
      <div className="h-full flex items-center justify-center p-8">
        <ErrorAlert error={error} title="Failed to load events" className="max-w-md" />
      </div>
    )
  }

  if (isLoading && !events.length) {
    return (
      <div className="h-full flex items-center justify-center">
        <div className="flex flex-col items-center gap-2 text-muted-foreground">
          <Loader2 className="h-6 w-6 animate-spin" />
          <p className="text-sm">Loading events...</p>
        </div>
      </div>
    )
  }

  if (!events.length) {
    return (
      <div className="h-full flex items-center justify-center p-8">
        <div className="text-center space-y-4 max-w-md">
          <div className="text-muted-foreground">
            <FileText className="mx-auto h-12 w-12 opacity-50" />
          </div>
          <div className="space-y-2">
            <h3 className="text-lg font-medium">No events yet</h3>
            <p className="text-sm text-muted-foreground">
              Events will appear here as the task progresses.
            </p>
          </div>
        </div>
      </div>
    )
  }

  return (
    <ScrollArea className="h-full w-full" ref={scrollRef}>
      <div className="p-4">
        <EventList events={mergedEvents} />
      </div>
    </ScrollArea>
  )
}
