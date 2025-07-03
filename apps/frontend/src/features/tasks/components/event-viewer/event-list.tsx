import { isToolUseContent } from '@moatless/shared'
import type { Event, MessageEvent as MessageEventType } from '@moatless/shared/types/event'
import React from 'react'
import type { MergedEvent } from '@/features/tasks/utils/merge-tool-events'

import { InputEvent, MessageEvent, ResultEvent, WorkspaceEvent } from './event-components'
import { ToolCallEvent } from './event-components/tool-call-event'

interface EventListProps {
  events: Array<Event | MergedEvent>
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

// Helper to check if an event contains tool calls
function hasToolCall(event: Event): boolean {
  if (event.type !== 'message') return false

  const messageEvent = event as MessageEventType
  const payload = messageEvent.payload

  // Check if it's an assistant message with content
  if (
    payload?.type === 'assistant' &&
    payload.message?.content &&
    Array.isArray(payload.message.content)
  ) {
    return payload.message.content.some(isToolUseContent)
  }

  return false
}

const EventContent = ({ event }: { event: Event | MergedEvent }) => {
  try {
    switch (event.type) {
      case 'workspace':
        return <WorkspaceEvent event={event} />
      case 'input':
        return <InputEvent event={event} />
      case 'message':
        // Check if it's a merged event with tool result
        if ('toolResult' in event) {
          return <ToolCallEvent event={event as MergedEvent} />
        }

        // Check if it's a tool call by examining the content
        if (hasToolCall(event)) {
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
            {event.subType && <p>Sub Type: {event.subType}</p>}
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
        <div className="bg-status-error/10 border border-status-error/20 rounded-lg p-3">
          <p className="text-status-error font-medium mb-2">
            Failed to render {event.type} event
          </p>
          <p className="text-xs text-status-error">
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

function EventListComponent({ events }: EventListProps) {
  // Events are already merged by the parent component
  return (
    <div className="space-y-2 w-full max-w-[48rem] min-w-[24rem] mx-auto">
      {events.map((event) => (
        <EventContent key={event.eventId} event={event} />
      ))}
    </div>
  )
}

export const EventList = React.memo(EventListComponent)
