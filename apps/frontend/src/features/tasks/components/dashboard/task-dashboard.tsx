import type { Task } from '@moatless/types/types/task'
import { useMemo } from 'react'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { TaskCard } from '@/features/tasks/components/task-card'
import { useTasksList } from '@/features/tasks/hooks/use-tasks'
import { ErrorAlert } from '@/lib/components/error-alert'

function TaskDashboardSkeleton() {
  return (
    <div className="container mx-auto max-w-7xl p-6">
      <div className="h-8 w-48 bg-muted animate-pulse rounded mb-6" />
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
        {Array.from({ length: 8 }, (_, i) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: Static skeleton items don't change order
          <div key={i} className="h-40 bg-muted animate-pulse rounded-lg" />
        ))}
      </div>
    </div>
  )
}

function EmptyTaskState() {
  return (
    <div className="container mx-auto max-w-7xl p-6">
      <h1 className="text-2xl font-bold mb-6">Task Dashboard</h1>
      <div className="flex flex-col items-center justify-center py-12 text-center">
        <div className="w-24 h-24 bg-muted rounded-full mb-4" />
        <h3 className="text-lg font-medium mb-2">No tasks yet</h3>
        <p className="text-muted-foreground mb-4">Create your first task to get started</p>
      </div>
    </div>
  )
}

function TaskDashboardHeader({ taskCount }: { taskCount: number }) {
  return (
    <div className="flex items-center justify-between mb-6">
      <div>
        <h1 className="text-2xl font-bold">Task Dashboard</h1>
        <p className="text-muted-foreground">
          {taskCount} {taskCount === 1 ? 'task' : 'tasks'}
        </p>
      </div>
    </div>
  )
}

export function TaskDashboard() {
  // Fetch tasks with higher limit for dashboard
  const {
    data: tasksData,
    isLoading: isLoadingTasks,
    error: tasksError,
  } = useTasksList(
    { limit: 50 },
    {
      // Real-time updates handled by global event stream
      refetchInterval: false,
    },
  )

  // Fetch repositories for name mapping
  const { data: repositories } = useRepositoriesList()

  // Create repository lookup map (derived state)
  const repositoryMap = useMemo(() => {
    if (!repositories) return {}
    return repositories.reduce(
      (acc, repo) => {
        acc[repo.id] = repo.name
        return acc
      },
      {} as Record<string, string>,
    )
  }, [repositories])

  // Sort tasks by createdAt descending (derived state)
  const sortedTasks = useMemo(() => {
    if (!tasksData?.items) return []
    return [...tasksData.items].sort(
      (a: Task, b: Task) => new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime(),
    )
  }, [tasksData?.items])

  if (tasksError) {
    return (
      <div className="container mx-auto max-w-7xl p-6">
        <h1 className="text-2xl font-bold mb-6">Task Dashboard</h1>
        <ErrorAlert error={tasksError} />
      </div>
    )
  }

  if (isLoadingTasks) {
    return <TaskDashboardSkeleton />
  }

  if (!sortedTasks.length) {
    return <EmptyTaskState />
  }

  return (
    <div className="container mx-auto max-w-7xl p-6">
      <TaskDashboardHeader taskCount={sortedTasks.length} />
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
        {sortedTasks.map((task: Task) => {
          const repositoryName = repositoryMap[task.repositoryId] || task.repositoryId
          return <TaskCard key={task.taskId} task={task} repositoryName={repositoryName} />
        })}
      </div>
    </div>
  )
}
