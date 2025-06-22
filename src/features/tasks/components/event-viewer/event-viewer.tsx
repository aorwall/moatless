import { useParams } from '@tanstack/react-router';
import { FileText, Loader2 } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useTaskEventsWithStreaming } from '@/features/tasks/hooks/use-task-events-with-streaming';
import { ErrorAlert } from '@/lib/components/ui/error-alert';
import { ScrollArea } from '@/lib/components/ui/scroll-area';

import { EventList } from './event-list';

// Track visited tasks to determine if this is the first time opening this task
const visitedTasks = new Set<string>();

export function EventViewer() {
  const params = useParams({ from: '/tasks/$taskId' });
  const taskId = params.taskId;

  // Use the abstraction that handles streaming automatically
  const { events, isLoading, error, isStreaming } = useTaskEventsWithStreaming(taskId);

  const scrollAreaRef = useRef<HTMLDivElement>(null);
  const [isAtBottom, setIsAtBottom] = useState(true);
  const previousEventCount = useRef(events.length);
  const hasScrolledToBottomInitially = useRef(false);

  // Scroll to bottom on initial load if task hasn't been visited before
  useEffect(() => {
    if (
      !hasScrolledToBottomInitially.current &&
      scrollAreaRef.current &&
      events &&
      events.length > 0
    ) {
      const isFirstVisit = !visitedTasks.has(taskId);

      if (isFirstVisit) {
        const scrollElement = scrollAreaRef.current.querySelector(
          '[data-slot="scroll-area-viewport"]',
        );
        if (scrollElement) {
          // Use requestAnimationFrame to ensure DOM has rendered
          requestAnimationFrame(() => {
            scrollElement.scrollTop = scrollElement.scrollHeight;
            hasScrolledToBottomInitially.current = true;
          });
        }
        visitedTasks.add(taskId);
      } else {
        hasScrolledToBottomInitially.current = true;
      }
    }
  }, [events, taskId]);

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

  if (isLoading) {
    return (
      <div className="h-full flex items-center justify-center p-6">
        <div className="text-center space-y-3">
          <Loader2 className="h-8 w-8 mx-auto text-muted-foreground/50 animate-spin" />
          <p className="text-muted-foreground">Loading events...</p>
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="h-full flex items-center justify-center p-6">
        <ErrorAlert
          error={error}
          title="Failed to load events"
          className="max-w-md"
        />
      </div>
    );
  }

  if (events.length === 0) {
    return (
      <div className="h-full flex items-center justify-center p-6">
        <div className="text-center space-y-3">
          <FileText className="h-8 w-8 mx-auto text-muted-foreground/50" />
          <p className="text-muted-foreground">
            {isStreaming ? 'Waiting for events...' : 'No events to display'}
          </p>
        </div>
      </div>
    );
  }

  return (
    <ScrollArea ref={scrollAreaRef} className="h-full w-full">
      <div className="p-2 sm:p-3 lg:p-4 w-full">
        <EventList events={events} />
      </div>
    </ScrollArea>
  );
}
