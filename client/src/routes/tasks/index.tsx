import { createFileRoute } from '@tanstack/react-router'

import { TaskList } from '@/features/tasks/components/task-list'
import { TaskListSkeleton } from '@/features/tasks/components/task-skeletons'
import { useTasksList } from '@/features/tasks/hooks/use-tasks'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import type { Task } from '@/types/task'

export const Route = createFileRoute('/tasks/')({
  component: TasksIndex,
})

function TasksIndex() {
  // Fetch all tasks
  const {
    data: tasks,
    isLoading,
    error,
  } = useTasksList(
    {},
    {
      refetchInterval: 5000,
      refetchIntervalInBackground: true,
    },
  )

  // Transform and separate tasks
  const transformedTasks: Array<Task> =
    tasks?.map((metadata) => ({
      id: metadata.task_id,
      title: metadata.title || `Task ${metadata.task_id.slice(0, 8)}`,
      createdAt: metadata.created_at,
      lastMessageAt: metadata.last_message_at,
      agentType: metadata.agent_type,
      environmentName: metadata.environment_id,
      status: metadata.status,
      numTurns: metadata.num_turns,
      totalCostUsd: metadata.total_cost_usd,
      workspaceStatus: 'unknown' as const,
      // Legacy fields for backward compatibility
      time: new Date(metadata.last_message_at).toLocaleString(),
      project: metadata.agent_type,
      changes: { added: 0, removed: 0 },
    })) || []

  const activeTasks = transformedTasks.filter((task) => task.status === 'active')

  const finishedTasks = transformedTasks.filter((task) => task.status !== 'active')

  return (
    <div className="container mx-auto px-6 py-8">
      <div className="mx-auto max-w-4xl">
        <div className="mb-8">
          <h1 className="text-3xl font-bold">All Tasks</h1>
          <p className="text-muted-foreground mt-2">
            View and manage all your active and archived tasks
          </p>
        </div>
        {error ? (
          <ErrorAlert error={error} title="Failed to load tasks" />
        ) : isLoading ? (
          <TaskListSkeleton />
        ) : (
          <TaskList activeTasks={activeTasks} archivedTasks={finishedTasks} loading={false} />
        )}
      </div>
    </div>
  )
}
