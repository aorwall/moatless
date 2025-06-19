import { createFileRoute } from '@tanstack/react-router';

import { ChatInput } from '@/features/tasks/components/chat-input';
import { TaskList } from '@/features/tasks/components/task-list';
import { TaskListSkeleton } from '@/features/tasks/components/task-skeletons';
import { useTasksList } from '@/features/tasks/hooks/use-tasks';
import { ErrorAlert } from '@/lib/components/ui/error-alert';

export const Route = createFileRoute('/')({
  component: Dashboard,
});

function Dashboard() {
  // Fetch all tasks (both active and finished)
  const {
    data: tasks,
    isLoading,
    error,
  } = useTasksList(
    {}, // Remove activeOnly filter to get all tasks
    {
      refetchInterval: 5000, // Poll every 5 seconds
      refetchIntervalInBackground: true,
    },
  );

  // Transform and separate tasks into active and finished
  const transformedTasks =
    tasks?.map((metadata) => ({
      id: metadata.task_id,
      title: `Task ${metadata.task_id.slice(0, 8)}`,
      time: new Date(metadata.last_message_at).toLocaleString(),
      project: metadata.agent_type,
      status: metadata.status,
      changes: { added: 0, removed: 0 },
    })) || [];

  const activeTasks = transformedTasks.filter(
    (task) => task.status === 'active',
  );

  const finishedTasks = transformedTasks.filter(
    (task) => task.status !== 'active',
  );

  return (
    <div className="container mx-auto px-6 py-8">
      <div className="mx-auto max-w-4xl space-y-8">
        <ChatInput
          showEnvironmentSelector={true}
          showTitle={true}
          className="text-center space-y-4"
        />
        {error ? (
          <ErrorAlert error={error} title="Failed to load tasks" />
        ) : isLoading ? (
          <TaskListSkeleton />
        ) : (
          <TaskList
            activeTasks={activeTasks}
            archivedTasks={finishedTasks}
            loading={false}
          />
        )}
      </div>
    </div>
  );
}
