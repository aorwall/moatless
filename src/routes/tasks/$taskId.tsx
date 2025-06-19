import { createFileRoute } from '@tanstack/react-router';
import { useCallback, useEffect, useMemo, useState } from 'react';

import { ChatInput } from '@/features/tasks/components/chat-input';
import { EventViewer } from '@/features/tasks/components/event-viewer';
import { GitDiffViewer } from '@/features/tasks/components/git-diff-viewer';
import { TaskHeader } from '@/features/tasks/components/task-header';
import { TaskDetailSkeleton } from '@/features/tasks/components/task-skeletons';
import { TaskViewer } from '@/features/tasks/components/task-viewer';
import { useTaskSSE } from '@/features/tasks/hooks/use-sse';
import {
  useTask,
  useTaskEvents,
  useTaskGitDiff,
} from '@/features/tasks/hooks/use-tasks';
import { ErrorAlert } from '@/lib/components/ui/error-alert';
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from '@/lib/components/ui/resizable';
import {
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@/lib/components/ui/tabs';
import type { Event } from '@/types/agent-events';
import type { BaseMessage, ClaudeMessage, TaskIteration } from '@/types/task';

export const Route = createFileRoute('/tasks/$taskId')({
  component: TaskDetail,
});

function TaskDetail() {
  const { taskId } = Route.useParams();
  const [, setLocalMessages] = useState<Array<ClaudeMessage>>([]);
  const [streamingError, setStreamingError] = useState<string | null>(null);
  const [selectedIteration, setSelectedIteration] =
    useState<TaskIteration | null>(null);
  const [rightPanelView, setRightPanelView] = useState<'events' | 'diff'>(
    'events',
  );
  const [streamEvents, setStreamEvents] = useState<Array<Event>>([]);

  // Use React Query hooks
  const {
    data: metadata,
    isLoading: isLoadingMetadata,
    error: metadataError,
  } = useTask(taskId);
  const {
    data: events,
    isLoading: isLoadingEvents,
    error: eventsError,
  } = useTaskEvents(taskId);
  const {
    data: gitDiff,
    isLoading: isLoadingGitDiff,
    error: gitDiffError,
  } = useTaskGitDiff(taskId);

  const loading = isLoadingMetadata || isLoadingEvents || isLoadingGitDiff;
  const error = metadataError || eventsError || gitDiffError;

  // Memoize SSE callbacks to prevent reconnections
  const handleSSEMessage = useCallback((newMessage: BaseMessage) => {
    // Add message to local state for immediate UI update
    setLocalMessages((prev) => [...prev, newMessage as ClaudeMessage]);
    setStreamingError(null); // Clear error on successful message
  }, []);

  const handleSSEEvent = useCallback((newEvent: Event) => {
    // Add event to stream events
    setStreamEvents((prev) => [...prev, newEvent]);
    setStreamingError(null); // Clear error on successful event
  }, []);

  const handleSSEError = useCallback((error: Error) => {
    console.error('SSE error:', error);
    // If task not found, disable streaming
    if (error.message.includes('Task not found')) {
      setStreamingEnabled(false);
      setStreamingError('Task not found');
    } else if (error.message.includes('connection failed')) {
      setStreamingError('Connection failed after multiple attempts');
    } else {
      setStreamingError('Streaming connection error');
    }
  }, []);

  // Extract task status as a stable value
  const taskStatus = metadata?.status;

  // Initialize stream events with existing events when task is running
  useEffect(() => {
    if (metadata?.is_running && events && streamEvents.length === 0) {
      setStreamEvents(events);
    }
  }, [metadata?.is_running, events, streamEvents.length]);

  // Memoize autoConnect to prevent unnecessary effect triggers
  const autoConnect = useMemo(() => {
    return metadata?.is_running && taskStatus === 'active';
  }, [metadata?.is_running, taskStatus]);

  // Use SSE for real-time updates only if task exists and streaming is enabled
  useTaskSSE({
    taskId,
    autoConnect,
    onMessage: handleSSEMessage,
    onEvent: handleSSEEvent,
    onError: handleSSEError,
  });

  // Handle iteration click
  const handleIterationClick = (iteration: TaskIteration) => {
    setSelectedIteration(iteration);
    setRightPanelView('events');
  };

  if (error) {
    return (
      <div className="min-h-screen flex items-center justify-center p-4">
        <ErrorAlert
          error={error}
          title="Failed to load task"
          className="max-w-md w-full"
        />
      </div>
    );
  }

  if (loading) {
    return <TaskDetailSkeleton />;
  }

  if (!metadata) {
    return (
      <div className="min-h-screen flex items-center justify-center p-4">
        <ErrorAlert
          error={new Error(`Task with ID ${taskId} could not be found`)}
          title="Task not found"
          className="max-w-md w-full"
        />
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full w-full absolute inset-0">
      <TaskHeader metadata={metadata} streamingError={streamingError} />
      <ResizablePanelGroup direction="horizontal" className="flex-1 w-full">
        {/* Left side - Task Viewer */}
        <ResizablePanel defaultSize={50} minSize={30}>
          <div className="h-full flex flex-col border-r">
            <div className="border-b px-4 py-2">
              <h2 className="text-sm font-medium">Task</h2>
            </div>
            <div className="flex-1 overflow-hidden">
              <TaskViewer
                metadata={metadata}
                onIterationClick={handleIterationClick}
              />
            </div>
            <ChatInput
              taskId={taskId}
              taskStatus={metadata.status}
              className="p-4 border-t border-slate-200 dark:border-slate-700"
            />
          </div>
        </ResizablePanel>

        <ResizableHandle withHandle />

        {/* Right side - Tabs with Events and Diff */}
        <ResizablePanel defaultSize={50} minSize={30}>
          <div className="h-full flex flex-col">
            <Tabs
              value={rightPanelView}
              onValueChange={(value) =>
                setRightPanelView(value as 'events' | 'diff')
              }
              className="h-full w-full flex flex-col"
            >
              <TabsList className="grid w-full grid-cols-2 mx-auto max-w-md">
                <TabsTrigger value="events">Events</TabsTrigger>
                <TabsTrigger value="diff">Diff</TabsTrigger>
              </TabsList>
              <TabsContent
                value="events"
                className="flex-1 w-full overflow-hidden mt-0"
              >
                <EventViewer
                  events={metadata.is_running ? streamEvents : events || []}
                  taskId={taskId}
                  isTaskRunning={metadata.is_running}
                />
              </TabsContent>
              <TabsContent
                value="diff"
                className="flex-1 w-full overflow-hidden mt-0"
              >
                <GitDiffViewer
                  gitDiff={selectedIteration?.git_diff || gitDiff}
                />
              </TabsContent>
            </Tabs>
          </div>
        </ResizablePanel>
      </ResizablePanelGroup>
    </div>
  );
}
