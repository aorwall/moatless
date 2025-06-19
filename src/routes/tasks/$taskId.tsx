import { createFileRoute } from '@tanstack/react-router';
import { useState } from 'react';

import { ChatInput } from '@/features/tasks/components/chat-input';
import { DiffViewer } from '@/features/tasks/components/diff-viewer';
import { EventViewer } from '@/features/tasks/components/event-viewer';
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
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from '@/lib/components/ui/tabs';
import type { ClaudeMessage, TaskIteration } from '@/types/task';

export const Route = createFileRoute('/tasks/$taskId')({
  component: TaskDetail,
});

function TaskDetail() {
  const { taskId } = Route.useParams();
  const [, setLocalMessages] = useState<Array<ClaudeMessage>>([]);
  const [streamingEnabled, setStreamingEnabled] = useState(false);
  const [streamingError, setStreamingError] = useState<string | null>(null);
  const [selectedIteration, setSelectedIteration] =
    useState<TaskIteration | null>(null);
  const [rightPanelView, setRightPanelView] = useState<'events' | 'diff'>(
    'events',
  );

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

  // Use SSE for real-time updates only if task exists and streaming is enabled
  const { connect, disconnect, isConnected } = useTaskSSE({
    taskId,
    autoConnect: streamingEnabled && !!metadata && metadata.status === 'active',
    onMessage: (newMessage) => {
      // Add message to local state for immediate UI update
      setLocalMessages((prev) => [...prev, newMessage as ClaudeMessage]);
      setStreamingError(null); // Clear error on successful message
    },
    onError: (error) => {
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
    },
  });

  // Handle streaming toggle
  const handleStreamingToggle = () => {
    if (streamingEnabled) {
      disconnect();
      setStreamingEnabled(false);
    } else {
      setStreamingEnabled(true);
      if (metadata && metadata.status === 'active') {
        connect();
      }
    }
  };

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
    <div className="flex flex-col h-screen">
      <TaskHeader
        metadata={metadata}
        streamingEnabled={streamingEnabled}
        isConnected={isConnected}
        streamingError={streamingError}
        onStreamingToggle={handleStreamingToggle}
      />
      <div className="flex flex-1 overflow-hidden">
        {/* Left side - Task Viewer */}
        <div className="w-1/2 border-r">
          <div className="h-full flex flex-col">
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
              disabled={metadata.status !== 'active'}
              placeholder={
                metadata.status !== 'active'
                  ? `Task is ${metadata.status}. Cannot send new messages.`
                  : 'Type a new request...'
              }
            />
          </div>
        </div>

        {/* Right side - Tabs with Events and Diff */}
        <div className="w-1/2">
          <Tabs
            value={rightPanelView}
            onValueChange={(value) =>
              setRightPanelView(value as 'events' | 'diff')
            }
            className="h-full flex flex-col"
          >
            <TabsList className="grid w-full grid-cols-2">
              <TabsTrigger value="events">Events</TabsTrigger>
              <TabsTrigger value="diff">Diff</TabsTrigger>
            </TabsList>
            <TabsContent value="events" className="flex-1 overflow-hidden">
              <EventViewer events={events || []} />
            </TabsContent>
            <TabsContent value="diff" className="flex-1 overflow-hidden">
              <DiffViewer gitDiff={selectedIteration?.git_diff || gitDiff} />
            </TabsContent>
          </Tabs>
        </div>
      </div>
    </div>
  );
}
