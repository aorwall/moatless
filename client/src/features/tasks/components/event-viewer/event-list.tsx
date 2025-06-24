import { useMemo } from 'react'

import { type MergedEvent, mergeToolEvents } from '@/features/tasks/utils/merge-tool-events'
import type { Event } from '@/types/event.ts'

import { InputEvent, MessageEvent, ResultEvent, WorkspaceEvent } from './event-components'
import { ToolCallEvent } from './event-components/tool-call-event'

interface EventListProps {
  events: Array<Event>
}

const PayloadRenderer = ({ payload }: { payload: unknown }) => {
  return (
    <div className="bg-muted rounded-lg p-4 mt-3 w-full">
      <pre className="text-xs text-muted-foreground overflow-x-auto whitespace-pre-wrap font-mono leading-relaxed break-words">
        {JSON.stringify(payload, null, 2)}
      </pre>
    </div>
  )
}

const EventContent = ({ event }: { event: Event | MergedEvent }) => {
  try {
    switch (event.type) {
      case 'workspace':
        return <WorkspaceEvent event={event} />
      case 'input':
        return <InputEvent event={event} />
      case 'message':
        // Check if it's a tool call with merged result
        if (event.sub_type === 'tool_call' && 'toolResult' in event) {
          return <ToolCallEvent event={event as MergedEvent} />
        }
        // Check if it's a standalone tool call (without result)
        if (event.sub_type === 'tool_call') {
          return <ToolCallEvent event={event as MergedEvent} />
        }
        // For text messages and other non-tool messages
        return <MessageEvent event={event} />
      case 'result':
        return <ResultEvent event={event} />
      default:
        return (
          <div className="text-sm text-muted-foreground">
            <p>Event Type: {event.type}</p>
            {event.sub_type && <p>Sub Type: {event.sub_type}</p>}
            <div className="mt-2">
              <PayloadRenderer payload={event.payload} />
            </div>
          </div>
        )
    }
  } catch (error) {
    console.error('Error rendering event:', error, event)
    return (
      <div className="text-sm text-muted-foreground">
        <div className="bg-red-50 dark:bg-red-950/20 border border-red-200 dark:border-red-800 rounded-lg p-3">
          <p className="text-red-600 dark:text-red-400 font-medium mb-2">
            Failed to render {event.type} event
          </p>
          <p className="text-xs text-red-500 dark:text-red-400">
            {error instanceof Error ? error.message : 'Unknown error'}
          </p>
        </div>
        <div className="mt-2">
          <PayloadRenderer payload={event.payload} />
        </div>
      </div>
    )
  }
}

export function EventList({ events }: EventListProps) {
  // Memoize sorted and merged events to prevent unnecessary re-computations
  const mergedEvents = useMemo(() => {
    const sortedEvents = [...events].sort(
      (a, b) => new Date(a.timestamp).getTime() - new Date(b.timestamp).getTime(),
    )
    return mergeToolEvents(sortedEvents)
  }, [events])

  return (
    <div className="space-y-2 w-full max-w-[48rem] min-w-[24rem] mx-auto">
      {mergedEvents.map((event) => (
        <EventContent key={event.event_id} event={event} />
      ))}
    </div>
  )
}
