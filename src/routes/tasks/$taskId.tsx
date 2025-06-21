import { createFileRoute } from '@tanstack/react-router';

import { EventViewer } from '@/features/tasks/components/event-viewer';
import { GitDiffViewer } from '@/features/tasks/components/git-diff-viewer';
import { TaskChatInput } from '@/features/tasks/components/task-chat-input';
import { TaskHeader } from '@/features/tasks/components/task-header';
import { TaskDetailSkeleton } from '@/features/tasks/components/task-skeletons';
import { TaskViewer } from '@/features/tasks/components/task-viewer';
import {
  TaskProvider,
  useTaskContext,
} from '@/features/tasks/contexts/task-context';
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

export const Route = createFileRoute('/tasks/$taskId')({
  component: TaskDetailWrapper,
});

function TaskDetailWrapper() {
  return (
    <TaskProvider>
      <TaskDetail />
    </TaskProvider>
  );
}

function TaskDetail() {
  const {
    taskId,
    metadata,
    isLoadingMetadata,
    metadataError,
    setRightPanelView,
    rightPanelView,
  } = useTaskContext();

  // Only consider critical errors that should break the whole page
  const loading = isLoadingMetadata;
  const error = metadataError; // Only metadata error is critical

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
      <TaskHeader />
      <ResizablePanelGroup direction="horizontal" className="flex-1 w-full">
        {/* Left side - Task Viewer */}
        <ResizablePanel defaultSize={50} minSize={30}>
          <div className="h-full flex flex-col border-r">
            <div className="border-b px-4 py-2">
              <h2 className="text-sm font-medium">Task</h2>
            </div>
            <div className="flex-1 overflow-hidden">
              <TaskViewer />
            </div>
            <TaskChatInput className="p-4 border-t border-slate-200 dark:border-slate-700" />
          </div>
        </ResizablePanel>

        <ResizableHandle withHandle />

        {/* Right side - Tabs with Events and Diff */}
        <ResizablePanel defaultSize={50} minSize={20}>
          <div className="h-full flex flex-col">
            <Tabs
              value={rightPanelView}
              onValueChange={(value) =>
                setRightPanelView(value as 'events' | 'diff')
              }
              className="h-full w-full flex flex-col"
            >
              <TabsList className="grid w-full grid-cols-2 mx-auto">
                <TabsTrigger value="events">Events</TabsTrigger>
                <TabsTrigger value="diff">Diff</TabsTrigger>
              </TabsList>
              <TabsContent
                value="events"
                className="flex-1 w-full overflow-hidden mt-0"
              >
                <EventViewer />
              </TabsContent>
              <TabsContent
                value="diff"
                className="flex-1 w-full overflow-hidden mt-0"
              >
                <GitDiffViewer />
              </TabsContent>
            </Tabs>
          </div>
        </ResizablePanel>
      </ResizablePanelGroup>
    </div>
  );
}
