import { createFileRoute } from '@tanstack/react-router';
import { useEffect } from 'react';

import { EventViewer } from '@/features/tasks/components/event-viewer';
import { GitDiffViewer } from '@/features/tasks/components/git-diff-viewer';
import { TaskChatInput } from '@/features/tasks/components/task-chat-input';
import { TaskHeader } from '@/features/tasks/components/task-header';
import { TaskDetailSkeleton } from '@/features/tasks/components/task-skeletons';
import { TaskViewer } from '@/features/tasks/components/task-viewer';
import { useTaskSSEConnection } from '@/features/tasks/hooks/use-task-sse-connection';
import { useTask } from '@/features/tasks/hooks/use-tasks';
import {
  useRightPanelView,
  useTaskUIStore,
} from '@/features/tasks/stores/task-ui-store';
import { ErrorAlert } from '@/lib/components/ui/error-alert';
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from '@/lib/components/ui/resizable';
import {
  Tabs,
  TabsContent,
} from '@/lib/components/ui/tabs';
import { cn } from '@/lib/utils';

export const Route = createFileRoute('/tasks/$taskId')({
  component: TaskDetail,
});

function TaskDetail() {
  const { taskId } = Route.useParams();
  const { data: metadata, isLoading, error } = useTask(taskId);
  const rightPanelView = useRightPanelView();
  const setRightPanelView = useTaskUIStore((state) => state.setRightPanelView);
  const resetUIState = useTaskUIStore((state) => state.resetUIState);

  // Set up SSE connection
  useTaskSSEConnection({ taskId });

  // Reset UI state when component unmounts or task changes
  useEffect(() => {
    return () => {
      resetUIState();
    };
  }, [taskId, resetUIState]);

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

  if (isLoading) {
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
            <div className="flex-1 overflow-hidden">
              <TaskViewer />
            </div>
            <TaskChatInput className="p-4 border-slate-200 dark:border-slate-700" />
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
              <div className="px-4 py-2 border-b">
                <div className="relative flex w-fit min-w-0 flex-1 items-center gap-2 overflow-x-auto">
                  <button
                    onClick={() => setRightPanelView('events')}
                    className={cn(
                      "font-regular group h-7 max-w-56 select-none whitespace-nowrap rounded-md px-2 text-sm font-medium transition-all cursor-pointer",
                      rightPanelView === 'events'
                        ? "bg-black/10 dark:bg-white/10 text-gray-900 dark:text-gray-100"
                        : "hover:bg-black/5 dark:hover:bg-white/5 bg-transparent text-gray-500 dark:text-gray-400"
                    )}
                    data-testid="tab-file-button"
                    data-active-tab={rightPanelView === 'events'}
                  >
                    <div className="truncate">
                      <div className="flex items-center gap-2">
                        <span>Events</span>
                      </div>
                    </div>
                  </button>
                  <button
                    onClick={() => setRightPanelView('diff')}
                    className={cn(
                      "font-regular group h-7 max-w-56 select-none whitespace-nowrap rounded-md px-2 text-sm font-medium transition-all cursor-pointer",
                      rightPanelView === 'diff'
                        ? "bg-black/10 dark:bg-white/10 text-gray-900 dark:text-gray-100"
                        : "hover:bg-black/5 dark:hover:bg-white/5 bg-transparent text-gray-500 dark:text-gray-400"
                    )}
                    data-testid="tab-file-button"
                    data-active-tab={rightPanelView === 'diff'}
                  >
                    <div className="truncate">
                      <div className="flex items-center gap-2">
                        <span>Diff</span>
                      </div>
                    </div>
                  </button>
                </div>
              </div>
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
