import type { TaskResponse } from '@moatless/api-client'
import { useMemo, useState } from 'react'
import { CreateCodeTask } from '@/features/tasks/components/create-task/create-code-task'
import {
  type DashboardSearchParams,
  TaskFilters,
} from '@/features/tasks/components/dashboard/task-filters'
import { TaskTable } from '@/features/tasks/components/dashboard/task-table'
import { TaskCard } from '@/features/tasks/components/task-card'
import { useTaskPreferences } from '@/features/tasks/hooks/use-task-preferences'
import { useTasksList } from '@/features/tasks/hooks/use-tasks'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { type ClaudeModel, DEFAULT_CLAUDE_MODEL } from '@/lib/types/claude-models'

const PAGE_SIZE = 20

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
  const preferences = useTaskPreferences()
  const [model, setModel] = useState<ClaudeModel>(DEFAULT_CLAUDE_MODEL)
  return (
    <div className="container mx-auto max-w-7xl p-6">
      {/* Create Task Section */}
      <div className="w-full max-w-3xl mx-auto mt-12 mb-12">
        <CreateCodeTask
          showTitle={true}
          title="What should we build today?"
          className="w-full"
          maxHeight="300px"
          minHeight="80px"
          model={model}
          onModelChange={setModel}
          agentMode={preferences.agentMode}
          onAgentModeChange={preferences.setAgentMode}
          repositoryId={preferences.repositoryId}
          onRepositoryChange={preferences.setRepositoryId}
        />
      </div>

      {/* Empty State Message */}
      <div className="flex flex-col items-center justify-center py-12 text-center">
        <div className="w-24 h-24 bg-muted rounded-full mb-4" />
        <h3 className="text-lg font-medium mb-2">No tasks yet</h3>
        <p className="text-muted-foreground mb-4">Create your first task above to get started</p>
      </div>
    </div>
  )
}

interface TaskDashboardProps {
  view: 'cards' | 'table'
  tag?: string
  createdBy?: string
  repositoryId?: string
  status?: 'open' | 'error' | 'closed'
  page: number
  onSearchChange: (params: DashboardSearchParams) => void
}

export function TaskDashboard({
  view,
  tag,
  createdBy,
  repositoryId,
  status,
  page,
  onSearchChange,
}: TaskDashboardProps) {
  const preferences = useTaskPreferences()
  const [model, setModel] = useState<ClaudeModel>(DEFAULT_CLAUDE_MODEL)

  const offset = (page - 1) * PAGE_SIZE

  // Fetch tasks with filters
  const {
    data: tasksData,
    isLoading: isLoadingTasks,
    error: tasksError,
  } = useTasksList(
    {
      limit: PAGE_SIZE,
      offset,
      status,
      tag,
      createdBy,
      repositoryId,
    },
    {
      refetchInterval: false,
    },
  )

  // Sort tasks by createdAt descending (derived state)
  const sortedTasks = useMemo(() => {
    if (!tasksData?.items) {
      return []
    }
    return [...tasksData.items].sort(
      (a: TaskResponse, b: TaskResponse) =>
        new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime(),
    )
  }, [tasksData?.items])

  const total = tasksData?.pagination?.total ?? 0

  function handlePageChange(newPage: number) {
    onSearchChange({
      view: view !== 'cards' ? view : undefined,
      tag,
      createdBy,
      repositoryId,
      status,
      page: newPage <= 1 ? undefined : newPage,
    })
  }

  if (tasksError) {
    return (
      <div className="container mx-auto max-w-7xl p-6">
        <h1 className="text-2xl font-bold mb-6">Task Dashboard</h1>
        <ErrorAlert error={tasksError} />
      </div>
    )
  }

  if (isLoadingTasks && !tasksData) {
    return <TaskDashboardSkeleton />
  }

  const hasFilters = tag || createdBy || repositoryId || status
  if (sortedTasks.length === 0 && !hasFilters) {
    return <EmptyTaskState />
  }

  return (
    <div className="container mx-auto max-w-7xl p-6">
      {/* Create Task Section */}
      <div className="w-full max-w-3xl mx-auto mt-26 mb-26">
        <CreateCodeTask
          showTitle={true}
          title="What should we build today?"
          className="w-full"
          maxHeight="300px"
          minHeight="80px"
          model={model}
          onModelChange={setModel}
          agentMode={preferences.agentMode}
          onAgentModeChange={preferences.setAgentMode}
          repositoryId={preferences.repositoryId}
          onRepositoryChange={preferences.setRepositoryId}
        />
      </div>

      {/* Tasks Section */}
      <div className="border-t pt-8">
        <div className="flex items-center justify-between mb-4 gap-4 flex-wrap">
          <h1 className="text-2xl font-bold">Tasks</h1>
          <TaskFilters
            view={view}
            tag={tag}
            createdBy={createdBy}
            repositoryId={repositoryId}
            status={status}
            onSearchChange={onSearchChange}
          />
        </div>

        {view === 'table' ? (
          <TaskTable
            tasks={sortedTasks}
            total={total}
            page={page}
            pageSize={PAGE_SIZE}
            onPageChange={handlePageChange}
          />
        ) : (
          <>
            <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
              {sortedTasks.map((task: TaskResponse) => {
                return (
                  <TaskCard
                    key={task.taskId}
                    task={task}
                    repositoryName={task.repositoryName || task.repositoryId}
                  />
                )
              })}
            </div>
            {/* Card view pagination */}
            {total > PAGE_SIZE && (
              <div className="flex items-center justify-center gap-2 mt-6">
                <button
                  type="button"
                  className="text-sm text-muted-foreground hover:text-foreground disabled:opacity-50"
                  onClick={() => handlePageChange(page - 1)}
                  disabled={page <= 1}
                >
                  Previous
                </button>
                <span className="text-sm text-muted-foreground">
                  Page {page} of {Math.ceil(total / PAGE_SIZE)}
                </span>
                <button
                  type="button"
                  className="text-sm text-muted-foreground hover:text-foreground disabled:opacity-50"
                  onClick={() => handlePageChange(page + 1)}
                  disabled={page >= Math.ceil(total / PAGE_SIZE)}
                >
                  Next
                </button>
              </div>
            )}
          </>
        )}

        {sortedTasks.length === 0 && hasFilters && (
          <div className="flex flex-col items-center justify-center py-12 text-center">
            <h3 className="text-lg font-medium mb-2">No tasks match your filters</h3>
            <p className="text-muted-foreground">Try adjusting your filter criteria</p>
          </div>
        )}
      </div>
    </div>
  )
}
