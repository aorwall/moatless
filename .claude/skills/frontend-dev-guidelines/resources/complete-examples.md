# Complete Examples

This document provides full, working examples of common frontend patterns in the Moatless Vibe application.

## Table of Contents
- [Simple CRUD Feature](#simple-crud-feature)
- [Real-time Data with SSE](#real-time-data-with-sse)
- [Modal Dialog with Form](#modal-dialog-with-form)
- [List with Filters and Sorting](#list-with-filters-and-sorting)
- [Error Boundary Implementation](#error-boundary-implementation)

## Simple CRUD Feature

Complete implementation of a simple CRUD feature for managing tasks.

### API Layer
```tsx
// src/features/tasks/api/use-tasks-query.ts
import { useQuery } from '@tanstack/react-query'
import { listTasks } from '@moatless/api-client/sandbox/tasks/tasks'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { queryKeys } from '@/lib/api/query-keys'

export function useTasksQuery() {
  return useQuery({
    queryKey: queryKeys.tasks.list(),
    queryFn: () => unwrapResponse(listTasks())
  })
}

// src/features/tasks/api/use-create-task-mutation.ts
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { createTask } from '@moatless/api-client/sandbox/tasks/tasks'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { queryKeys } from '@/lib/api/query-keys'
import type { CreateTaskRequest } from '@moatless/api-client'

export function useCreateTaskMutation() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (data: CreateTaskRequest) => unwrapResponse(createTask(data)),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: queryKeys.tasks.list()
      })
    }
  })
}
```

### Components
```tsx
// src/features/tasks/components/task-list.tsx
import { useTasksQuery } from '../api/use-tasks-query'
import { TaskCard } from './task-card'
import { LoadingSpinner } from '@/lib/components/loading-spinner'
import { ErrorDisplay } from '@/lib/components/error-display'
import { EmptyState } from '@/lib/components/empty-state'

export function TaskList() {
  const { data: tasks, isLoading, error, refetch } = useTasksQuery()

  if (isLoading) {
    return (
      <div className="flex justify-center p-8">
        <LoadingSpinner />
      </div>
    )
  }

  if (error) {
    return (
      <ErrorDisplay
        error={error}
        retry={refetch}
        message="Failed to load tasks"
      />
    )
  }

  if (!tasks || tasks.length === 0) {
    return (
      <EmptyState
        message="No tasks yet"
        action={{
          label: 'Create First Task',
          onClick: () => navigate({ to: '/tasks/new' })
        }}
      />
    )
  }

  return (
    <div className="space-y-4">
      {tasks.map(task => (
        <TaskCard key={task.id} task={task} />
      ))}
    </div>
  )
}

// src/features/tasks/components/task-card.tsx
import { Link } from '@tanstack/react-router'
import { cn } from '@/lib/utils'
import type { TaskResponse as Task } from '@moatless/api-client'

interface TaskCardProps {
  task: Task
}

export function TaskCard({ task }: TaskCardProps) {
  return (
    <Link
      to="/tasks/$taskId"
      params={{ taskId: task.id }}
      className="block border rounded-lg p-4 hover:shadow-md transition-shadow"
    >
      <div className="flex items-start justify-between">
        <div>
          <h3 className="text-lg font-semibold">{task.title}</h3>
          {task.description && (
            <p className="text-muted-foreground mt-1">{task.description}</p>
          )}
        </div>
        <span
          className={cn(
            'px-2 py-1 text-xs rounded-full',
            task.status === 'completed' && 'bg-muted text-status-success',
            task.status === 'running' && 'bg-muted text-status-info',
            task.status === 'pending' && 'bg-muted text-muted-foreground'
          )}
        >
          {task.status}
        </span>
      </div>
    </Link>
  )
}

// src/features/tasks/components/create-task-form.tsx
import { useState } from 'react'
import { useNavigate } from '@tanstack/react-router'
import { useCreateTaskMutation } from '../api/use-create-task-mutation'
import { useApiError } from '@/lib/hooks/use-api-error'
import { toast } from 'sonner'

export function CreateTaskForm() {
  const [title, setTitle] = useState('')
  const [description, setDescription] = useState('')

  const navigate = useNavigate()
  const createTask = useCreateTaskMutation()
  const { showError } = useApiError()

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()

    if (!title.trim()) {
      toast.error('Title is required')
      return
    }

    try {
      const newTask = await createTask.mutateAsync({
        title: title.trim(),
        description: description.trim()
      })

      toast.success('Task created successfully!')

      await navigate({
        to: '/tasks/$taskId',
        params: { taskId: newTask.id }
      })
    } catch (error) {
      showError(error)
    }
  }

  return (
    <form onSubmit={handleSubmit} className="space-y-4">
      <div>
        <label htmlFor="title" className="block text-sm font-medium mb-1">
          Task Title *
        </label>
        <input
          id="title"
          type="text"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          className="w-full px-3 py-2 border rounded-md focus:outline-none focus:ring-2 focus:ring-ring"
          placeholder="Enter task title"
          disabled={createTask.isPending}
        />
      </div>

      <div>
        <label htmlFor="description" className="block text-sm font-medium mb-1">
          Description
        </label>
        <textarea
          id="description"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          className="w-full px-3 py-2 border rounded-md focus:outline-none focus:ring-2 focus:ring-ring"
          rows={4}
          placeholder="Enter task description (optional)"
          disabled={createTask.isPending}
        />
      </div>

      <div className="flex gap-2">
        <button
          type="submit"
          disabled={createTask.isPending}
          className="px-4 py-2 bg-primary text-primary-foreground rounded-md hover:bg-primary/90 disabled:opacity-50"
        >
          {createTask.isPending ? 'Creating...' : 'Create Task'}
        </button>
        <button
          type="button"
          onClick={() => navigate({ to: '/tasks' })}
          className="px-4 py-2 border rounded-md hover:bg-muted"
        >
          Cancel
        </button>
      </div>
    </form>
  )
}
```

### Route
```tsx
// src/routes/tasks/new.tsx
import { createFileRoute } from '@tanstack/react-router'
import { CreateTaskForm } from '@/features/tasks/components/create-task-form'

export const Route = createFileRoute('/tasks/new')({
  component: NewTaskPage
})

function NewTaskPage() {
  return (
    <div className="container mx-auto p-4 max-w-2xl">
      <h1 className="text-2xl font-bold mb-6">Create New Task</h1>
      <CreateTaskForm />
    </div>
  )
}
```

## Modal Dialog with Form

Complete modal dialog implementation using shadcn/ui.

```tsx
// src/features/tasks/components/edit-task-dialog.tsx
import { useState } from 'react'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogTrigger
} from '@/lib/components/ui/dialog'
import { Button } from '@/lib/components/ui/button'
import { TaskForm } from './task-form'
import { useUpdateTaskMutation } from '../api/use-update-task-mutation'
import { useApiError } from '@/lib/hooks/use-api-error'
import { toast } from 'sonner'
import type { TaskResponse as Task } from '@moatless/api-client'

interface EditTaskDialogProps {
  task: Task
}

export function EditTaskDialog({ task }: EditTaskDialogProps) {
  const [open, setOpen] = useState(false)
  const updateTask = useUpdateTaskMutation(task.id)
  const { showError } = useApiError()

  const handleSubmit = async (data: TaskFormData) => {
    try {
      await updateTask.mutateAsync(data)
      toast.success('Task updated successfully!')
      setOpen(false)
    } catch (error) {
      showError(error)
    }
  }

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button variant="outline">Edit Task</Button>
      </DialogTrigger>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>Edit Task</DialogTitle>
        </DialogHeader>
        <TaskForm
          task={task}
          onSubmit={handleSubmit}
          onCancel={() => setOpen(false)}
        />
      </DialogContent>
    </Dialog>
  )
}
```

## List with Filters and Sorting

Complete list with filtering and sorting functionality.

```tsx
// src/features/tasks/components/task-list-with-filters.tsx
import { useState, useMemo } from 'react'
import { useTasksQuery } from '../api/use-tasks-query'
import { TaskCard } from './task-card'
import type { TaskResponse as Task, TaskStatus } from '@moatless/api-client'

type SortOption = 'date' | 'title' | 'priority'

export function TaskListWithFilters() {
  const [statusFilter, setStatusFilter] = useState<TaskStatus | 'all'>('all')
  const [sortBy, setSortBy] = useState<SortOption>('date')
  const [searchTerm, setSearchTerm] = useState('')

  const { data: tasks, isLoading, error } = useTasksQuery()

  // Filtered and sorted tasks
  const filteredTasks = useMemo(() => {
    if (!tasks) return []

    let result = [...tasks]

    // Filter by status
    if (statusFilter !== 'all') {
      result = result.filter(task => task.status === statusFilter)
    }

    // Filter by search term
    if (searchTerm) {
      const lowerSearch = searchTerm.toLowerCase()
      result = result.filter(task =>
        task.title.toLowerCase().includes(lowerSearch) ||
        task.description?.toLowerCase().includes(lowerSearch)
      )
    }

    // Sort
    result.sort((a, b) => {
      switch (sortBy) {
        case 'title':
          return a.title.localeCompare(b.title)
        case 'priority':
          const priorityOrder = { high: 0, medium: 1, low: 2 }
          return priorityOrder[a.priority] - priorityOrder[b.priority]
        case 'date':
        default:
          return new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime()
      }
    })

    return result
  }, [tasks, statusFilter, sortBy, searchTerm])

  if (isLoading) return <LoadingSpinner />
  if (error) return <ErrorDisplay error={error} />

  return (
    <div className="space-y-4">
      {/* Filters */}
      <div className="flex gap-4 flex-wrap">
        <input
          type="text"
          value={searchTerm}
          onChange={(e) => setSearchTerm(e.target.value)}
          placeholder="Search tasks..."
          className="px-3 py-2 border rounded-md flex-1 min-w-[200px]"
        />

        <select
          value={statusFilter}
          onChange={(e) => setStatusFilter(e.target.value as TaskStatus | 'all')}
          className="px-3 py-2 border rounded-md"
        >
          <option value="all">All Status</option>
          <option value="pending">Pending</option>
          <option value="running">Running</option>
          <option value="completed">Completed</option>
        </select>

        <select
          value={sortBy}
          onChange={(e) => setSortBy(e.target.value as SortOption)}
          className="px-3 py-2 border rounded-md"
        >
          <option value="date">Sort by Date</option>
          <option value="title">Sort by Title</option>
          <option value="priority">Sort by Priority</option>
        </select>
      </div>

      {/* Results */}
      <div className="text-sm text-muted-foreground">
        Showing {filteredTasks.length} of {tasks.length} tasks
      </div>

      {/* Task List */}
      {filteredTasks.length === 0 ? (
        <EmptyState message="No tasks found matching your filters" />
      ) : (
        <div className="space-y-4">
          {filteredTasks.map(task => (
            <TaskCard key={task.id} task={task} />
          ))}
        </div>
      )}
    </div>
  )
}
```

## Error Boundary Implementation

Complete error boundary with recovery mechanisms.

```tsx
// src/lib/components/error-boundary.tsx
import { Component, type ReactNode } from 'react'
import { AlertTriangle } from 'lucide-react'

interface Props {
  children: ReactNode
  fallback?: (error: Error, reset: () => void) => ReactNode
  onError?: (error: Error, errorInfo: React.ErrorInfo) => void
}

interface State {
  hasError: boolean
  error?: Error
}

export class ErrorBoundary extends Component<Props, State> {
  constructor(props: Props) {
    super(props)
    this.state = { hasError: false }
  }

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error }
  }

  componentDidCatch(error: Error, errorInfo: React.ErrorInfo) {
    console.error('Error caught by boundary:', error, errorInfo)
    this.props.onError?.(error, errorInfo)
  }

  handleReset = () => {
    this.setState({ hasError: false, error: undefined })
  }

  render() {
    if (this.state.hasError && this.state.error) {
      if (this.props.fallback) {
        return this.props.fallback(this.state.error, this.handleReset)
      }

      return (
        <div className="flex items-center justify-center min-h-[400px] p-4">
          <div className="max-w-md w-full bg-destructive/10 border border-destructive rounded-lg p-6">
            <div className="flex items-start gap-3">
              <AlertTriangle className="w-6 h-6 text-destructive mt-0.5" />
              <div className="flex-1">
                <h2 className="text-lg font-semibold text-destructive mb-2">
                  Something went wrong
                </h2>
                <p className="text-sm text-destructive mb-4">
                  {this.state.error.message || 'An unexpected error occurred'}
                </p>
                <div className="flex gap-2">
                  <button
                    onClick={this.handleReset}
                    className="px-4 py-2 bg-destructive text-destructive-foreground text-sm rounded hover:bg-destructive/90"
                  >
                    Try Again
                  </button>
                  <button
                    onClick={() => window.location.href = '/'}
                    className="px-4 py-2 border border-destructive text-destructive text-sm rounded hover:bg-destructive/10"
                  >
                    Go Home
                  </button>
                </div>
              </div>
            </div>
          </div>
        </div>
      )
    }

    return this.props.children
  }
}

// Usage Example
export function App() {
  return (
    <ErrorBoundary
      onError={(error, errorInfo) => {
        // Send to error tracking service
        console.error('Error:', error)
        console.error('Error Info:', errorInfo)
      }}
    >
      <RouterProvider router={router} />
    </ErrorBoundary>
  )
}
```
