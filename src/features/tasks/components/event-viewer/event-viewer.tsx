import { FileText } from 'lucide-react';
import { useEffect, useState } from 'react';

import { useEventSSE } from '@/features/tasks/hooks/use-event-sse';
import { ScrollArea } from '@/lib/components/ui/scroll-area';
import type { AgentEvent } from '@/types/agent-events';

import { EventList } from './event-list';

interface EventViewerProps {
  events: Array<AgentEvent>;
  taskId: string;
  isTaskRunning?: boolean;
}

export function EventViewer({
  events,
  taskId,
  isTaskRunning = false,
}: EventViewerProps) {
  const [localEvents, setLocalEvents] = useState<Array<AgentEvent>>(events);

  // Use SSE for real-time event updates only if task is running
  useEventSSE({
    taskId,
    enabled: isTaskRunning,
    onEvent: (newEvent) => {
      setLocalEvents((prev) => {
        // Check if event already exists to avoid duplicates
        const exists = prev.some((e) => e.event_id === newEvent.event_id);
        if (!exists) {
          return [...prev, newEvent];
        }
        return prev;
      });
    },
    onError: (error) => {
      console.error('Event SSE error:', error);
    },
  });

  // Update local events when props change
  useEffect(() => {
    setLocalEvents(events);
  }, [events]);
  const displayEvents = localEvents.length > 0 ? localEvents : events;

  if (!displayEvents || displayEvents.length === 0) {
    return (
      <div className="h-full flex items-center justify-center p-6">
        <div className="text-center space-y-3">
          <FileText className="h-8 w-8 mx-auto text-muted-foreground/50" />
          <p className="text-muted-foreground">No events to display</p>
        </div>
      </div>
    );
  }

  return (
    <ScrollArea className="h-full">
      <div className="p-6">
        <EventList events={displayEvents} />
      </div>
    </ScrollArea>
  );
}
