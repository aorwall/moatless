import { FileText } from 'lucide-react';

import { ScrollArea } from '@/lib/components/ui/scroll-area';
import type { AgentEvent } from '@/types/agent-events';

import { EventList } from './event-list';

interface EventViewerProps {
  events: Array<AgentEvent>;
}

export function EventViewer({ events }: EventViewerProps) {
  if (!events || events.length === 0) {
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
        <EventList events={events} />
      </div>
    </ScrollArea>
  );
}
