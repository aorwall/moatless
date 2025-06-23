import { ChevronRight } from 'lucide-react';

import type { Event } from '@/types/event.ts';

import {
    EventHeader,
    InputEvent,
    MessageEvent,
    ResultEvent,
    WorkspaceEvent,
} from './event-components';

interface EventListProps {
  events: Array<Event>;
}

const PayloadRenderer = ({ payload }: { payload: unknown }) => {
  return (
    <div className="bg-muted rounded-lg p-4 mt-3 w-full">
      <pre className="text-xs text-muted-foreground overflow-x-auto whitespace-pre-wrap font-mono leading-relaxed break-words">
        {JSON.stringify(payload, null, 2)}
      </pre>
    </div>
  );
};

const EventContent = ({ event }: { event: Event }) => {
  try {
    switch (event.type) {
      case 'workspace':
        return <WorkspaceEvent event={event} />;
      case 'input':
        return <InputEvent event={event} />;
      case 'message':
        return <MessageEvent event={event} />;
      case 'result':
        return <ResultEvent event={event} />;
      default:
        return (
          <EventHeader event={event}>
            <div className="text-sm text-muted-foreground">
              <p>Event Type: {event.type}</p>
              {event.sub_type && <p>Sub Type: {event.sub_type}</p>}
              <div className="mt-2">
                <PayloadRenderer payload={event.payload} />
              </div>
            </div>
          </EventHeader>
        );
    }
  } catch (error) {
    console.error('Error rendering event:', error, event);
    return (
      <EventHeader event={event}>
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
      </EventHeader>
    );
  }
};

const EventItem = ({ event }: { event: Event }) => {
  return (
    <div
      className={`w-full border rounded-lg transition-all duration-200 ${
        event.is_error ? 'border-border bg-muted/50' : 'border-border bg-card'
      }`}
    >
      <div className="p-4">
        {/* Event content with integrated header */}
        <EventContent event={event} />

        {/* Raw payload (collapsible) */}
        <details className="group mt-4">
          <summary className="cursor-pointer text-xs text-muted-foreground hover:text-foreground transition-colors">
            <span className="inline-flex items-center gap-1">
              <ChevronRight className="h-3 w-3 transition-transform group-open:rotate-90" />
              View raw payload
            </span>
          </summary>
          <div className="mt-2">
            <div className="text-xs text-muted-foreground font-mono mb-2 break-all">
              Event ID: {event.event_id}
            </div>
            <PayloadRenderer payload={event.payload} />
          </div>
        </details>
      </div>
    </div>
  );
};

export function EventList({ events }: EventListProps) {
  const sortedEvents = [...events].sort(
    (a, b) => new Date(a.timestamp).getTime() - new Date(b.timestamp).getTime(),
  );

  return (
    <div className="space-y-2 w-full">
      {sortedEvents.map((event) => (
        <EventItem key={event.event_id} event={event} />
      ))}
    </div>
  );
}
