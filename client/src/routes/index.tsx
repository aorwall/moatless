import { createFileRoute } from '@tanstack/react-router';

import { useEnvironmentsList } from '@/features/environments/hooks/use-environments';
import { CreateTask } from '@/features/tasks/components/create-task';
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
    isLoading: isLoadingTasks,
    error: tasksError,
  } = useTasksList(
    {}, // Remove activeOnly filter to get all tasks
    {
      refetchInterval: 5000, // Poll every 5 seconds
      refetchIntervalInBackground: true,
    },
  );

  // Fetch environments to map IDs to names
  const {
    data: environments,
    isLoading: isLoadingEnvironments,
    error: environmentsError,
  } = useEnvironmentsList();

  // Create a map of environment IDs to names
  const environmentMap = new Map(
    environments?.map((env) => [env.id, env.name]) || [],
  );

  // Transform and separate tasks into active and finished
  const transformedTasks =
    tasks?.map((metadata) => ({
      id: metadata.task_id,
      title: metadata.title || `Task ${metadata.task_id.slice(0, 8)}`,
      createdAt: metadata.created_at,
      lastMessageAt: metadata.last_message_at,
      agentType: metadata.agent_type,
      environmentName:
        environmentMap.get(metadata.environment_id) || metadata.environment_id,
      status: metadata.status,
      numTurns: metadata.num_turns,
      totalCostUsd: metadata.total_cost_usd,
      workspaceStatus: 'unknown' as const, // This will be fetched separately
      // Legacy fields for compatibility
      time: new Date(metadata.last_message_at).toLocaleString(),
      project: metadata.agent_type,
      changes: { added: 0, removed: 0 },
    })) || [];

  const activeTasks = transformedTasks.filter(
    (task) => task.status === 'active',
  );

  const finishedTasks = transformedTasks.filter(
    (task) => task.status !== 'active',
  );

  const error = tasksError || environmentsError;
  const isLoading = isLoadingTasks || isLoadingEnvironments;

  return (
    <div className="container mx-auto px-6 py-8">
      <div className="mx-auto max-w-4xl space-y-8">
        <CreateTask />
        {error ? (
          <ErrorAlert error={error} title="Failed to load data" />
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
