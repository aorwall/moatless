import { createFileRoute } from '@tanstack/react-router';

import { SearchSection } from '@/features/tasks/components/search-section';
import { TaskList } from '@/features/tasks/components/task-list';
import { useTasksList } from '@/features/tasks/hooks/use-tasks';

export const Route = createFileRoute('/')({
  component: Dashboard,
});

function Dashboard() {
  // Fetch all tasks (both active and finished)
  const { data: tasks, isLoading } = useTasksList(
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
        <SearchSection />
        <TaskList
          activeTasks={activeTasks}
          archivedTasks={finishedTasks}
          loading={isLoading}
        />
      </div>
    </div>
  );
}
