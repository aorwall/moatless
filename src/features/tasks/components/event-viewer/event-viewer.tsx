import { useParams } from '@tanstack/react-router';
import { FileText, Loader2 } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useTaskEventsWithStreaming } from '@/features/tasks/hooks/use-task-events';
import { ErrorAlert } from '@/lib/components/ui/error-alert';
import { ScrollArea } from '@/lib/components/ui/scroll-area';

import { EventList } from './event-list';

// Track visited tasks to determine if this is the first time opening this task
const visitedTasks = new Set<string>();

export function EventViewer() {
  const params = useParams({ from: '/tasks/$taskId' });
  const taskId = params.taskId;

  // Use the unified hook that handles streaming automatically
  const { events, isLoading, error, isStreaming } =
    useTaskEventsWithStreaming(taskId);

  const scrollAreaRef = useRef<HTMLDivElement>(null);
  const [isAtBottom, setIsAtBottom] = useState(true);
  const previousEventCount = useRef(events.length);
  const hasScrolledToBottomInitially = useRef(false);
  const userHasScrolled = useRef(false);

  // Scroll to bottom on initial load if task hasn't been visited before and user hasn't scrolled
  useEffect(() => {
    if (
      !(hasScrolledToBottomInitially.current || userHasScrolled.current) &&
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
            // Only scroll if user still hasn't manually scrolled
            if (!userHasScrolled.current) {
              scrollElement.scrollTop = scrollElement.scrollHeight;
              hasScrolledToBottomInitially.current = true;
            }
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

    // Mark that user has manually scrolled
    userHasScrolled.current = true;

    const { scrollTop, scrollHeight, clientHeight } = scrollElement;
    // Consider user at bottom if within 10px of the bottom
    const atBottom = Math.abs(scrollHeight - clientHeight - scrollTop) < 10;
    setIsAtBottom(atBottom);
  }, []);

  // Auto-scroll when new events arrive if at bottom
  useEffect(() => {
    // Only proceed if we have new events
    if (events.length <= previousEventCount.current) {
      previousEventCount.current = events.length;
      return;
    }

    // Only auto-scroll if user is already at bottom
    if (!(scrollAreaRef.current && isAtBottom)) {
      previousEventCount.current = events.length;
      return;
    }

    const scrollElement = scrollAreaRef.current.querySelector(
      '[data-slot="scroll-area-viewport"]',
    );
    if (scrollElement) {
      // Use requestAnimationFrame to ensure DOM has updated
      requestAnimationFrame(() => {
        // Double-check we're still at bottom before scrolling
        const { scrollTop, scrollHeight, clientHeight } = scrollElement;
        const stillAtBottom =
          Math.abs(scrollHeight - clientHeight - scrollTop) < 10;

        if (stillAtBottom) {
          scrollElement.scrollTop = scrollElement.scrollHeight;
        }
      });
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
          <Loader2 className="h-8 w-8 mx-auto text-muted-foreground animate-spin" />
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
          <FileText className="h-8 w-8 mx-auto text-muted-foreground" />
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
