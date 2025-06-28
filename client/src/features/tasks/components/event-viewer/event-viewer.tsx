import { useParams } from '@tanstack/react-router'
import { FileText, Loader2 } from 'lucide-react'

import { useAutoScroll } from '@/features/tasks/hooks/use-auto-scroll'
import { useTaskEventsContext } from '@/features/tasks/contexts/task-events-context'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { ScrollArea } from '@/lib/components/ui/scroll-area'

import { EventList } from './event-list'

// Track visited tasks to determine if this is the first time opening this task
const visitedTasks = new Set<string>()

export function EventViewer() {
  const params = useParams({ from: '/tasks/$taskId' })
  const taskId = params.taskId

  // Use the context to get events (single SSE connection shared across components)
  const { events, isLoading, error, isStreaming } = useTaskEventsContext()

  // Determine if this is the first visit to auto-scroll to bottom
  const isFirstVisit = !visitedTasks.has(taskId)
  if (events.length > 0 && isFirstVisit) {
    visitedTasks.add(taskId)
  }

  // Use auto-scroll hook to manage scroll behavior
  const { scrollRef } = useAutoScroll({
    scrollOnMount: isFirstVisit,
    dependencies: [events],
  })

  if (isLoading) {
    return (
      <div className="h-full flex items-center justify-center p-6">
        <div className="text-center space-y-3">
          <Loader2 className="h-8 w-8 mx-auto text-muted-foreground animate-spin" />
          <p className="text-muted-foreground">Loading events...</p>
        </div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="h-full flex items-center justify-center p-6">
        <ErrorAlert error={error} title="Failed to load events" className="max-w-md" />
      </div>
    )
  }

  if (events.length === 0) {
    return (
      <div className="h-full flex items-center justify-center p-6">
        <div className="text-center space-y-3">
          <FileText className="h-8 w-8 mx-auto text-muted-foreground" />
          <p className="text-muted-foreground">
            {isStreaming ? 'Waiting for events...' : 'No events to display'}
          </p>
        </div>
      </div>
    )
  }

  return (
    <ScrollArea ref={scrollRef} className="h-full w-full">
      <div className="p-2 sm:p-3 lg:p-4 w-full">
        <EventList events={events} />
      </div>
    </ScrollArea>
  )
}
