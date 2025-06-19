import { FileText } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';

import { ScrollArea } from '@/lib/components/ui/scroll-area';
import type { AgentEvent } from '@/types/agent-events';

import { EventList } from './event-list';

interface EventViewerProps {
  events: Array<AgentEvent>;
  taskId: string;
  isTaskRunning?: boolean;
}

export function EventViewer({ events }: EventViewerProps) {
  const [localEvents, setLocalEvents] = useState<Array<AgentEvent>>(events);
  const scrollAreaRef = useRef<HTMLDivElement>(null);
  const [isAtBottom, setIsAtBottom] = useState(true);
  const previousEventCount = useRef(events.length);

  // Update local events when props change
  useEffect(() => {
    setLocalEvents(events);
  }, [events]);

  // Track scroll position
  const handleScroll = useCallback(() => {
    if (!scrollAreaRef.current) {
      return;
    }

    const scrollElement = scrollAreaRef.current.querySelector(
      '[data-slot="scroll-area-viewport"]',
    );
    if (!scrollElement) {
      return;
    }

    const { scrollTop, scrollHeight, clientHeight } = scrollElement;
    const atBottom = Math.abs(scrollHeight - clientHeight - scrollTop) < 10;
    setIsAtBottom(atBottom);
  }, []);

  // Auto-scroll when new events arrive if at bottom
  useEffect(() => {
    if (!(scrollAreaRef.current && isAtBottom)) {
      return;
    }

    // Only auto-scroll if new events were added
    if (events.length > previousEventCount.current) {
      const scrollElement = scrollAreaRef.current.querySelector(
        '[data-slot="scroll-area-viewport"]',
      );
      if (scrollElement) {
        // Use requestAnimationFrame to ensure DOM has updated
        requestAnimationFrame(() => {
          scrollElement.scrollTop = scrollElement.scrollHeight;
        });
      }
    }

    previousEventCount.current = events.length;
  }, [events, isAtBottom]);

  // Set up scroll event listener
  useEffect(() => {
    if (!scrollAreaRef.current) {
      return;
    }

    const scrollElement = scrollAreaRef.current.querySelector(
      '[data-slot="scroll-area-viewport"]',
    );
    if (!scrollElement) {
      return;
    }

    scrollElement.addEventListener('scroll', handleScroll);

    // Check initial scroll position
    handleScroll();

    return () => {
      scrollElement.removeEventListener('scroll', handleScroll);
    };
  }, [handleScroll]);

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
    <ScrollArea ref={scrollAreaRef} className="h-full w-full">
      <div className="p-3 sm:p-4 lg:p-6 w-full">
        <EventList events={displayEvents} />
      </div>
    </ScrollArea>
  );
}
