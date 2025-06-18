import { ScrollArea } from '@/lib/components/ui/scroll-area';
import type { AgentEvent } from '@/types/agent-events';

import {
  type DisplayEvent,
  type ParsedEvent,
  useEventGroups,
} from '../hooks/use-claude-code-parser';
import { EventGroup } from './event-group';
import { EventRenderer } from './event-renderer';

interface LogsViewerProps {
  events: Array<AgentEvent>;
}

const getEventStyling = (
  type: ParsedEvent['type'],
  isError: boolean,
): string => {
  const baseStyle =
    'bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700';

  if (isError || type === 'error') {
    return `${baseStyle} border-red-300 dark:border-red-700`;
  }

  return baseStyle;
};

export function LogsViewer({ events }: LogsViewerProps) {
  const displayEvents = useEventGroups(events);

  // Early return if no events
  if (!events || events.length === 0) {
    return (
      <div className="h-full flex items-center justify-center p-6">
        <p className="text-muted-foreground">No events yet</p>
      </div>
    );
  }

  const renderDisplayEvent = (displayEvent: DisplayEvent) => {
    if (displayEvent.type === 'group') {
      return (
        <EventGroup
          key={`${displayEvent.inputEvent.event_id}-${displayEvent.resultEvent.event_id}`}
          inputEvent={displayEvent.inputEvent}
          resultEvent={displayEvent.resultEvent}
          intermediateEvents={displayEvent.intermediateEvents}
        />
      );
    } else {
      // Standalone event
      const event = displayEvent.event;
      return (
        <div
          key={event.event_id}
          className={`${getEventStyling(event.type, event.is_error)} shadow-sm p-4`}
        >
          <EventRenderer event={event} />
        </div>
      );
    }
  };

  return (
    <ScrollArea className="h-full">
      <div className="p-6 space-y-4">
        {displayEvents.map(renderDisplayEvent)}
      </div>
    </ScrollArea>
  );
}
