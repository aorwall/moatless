# Error Handling

## Table of Contents
- [Overview](#overview)
- [Error Boundaries](#error-boundaries)
- [API Error Handling](#api-error-handling)
- [Loading and Error States](#loading-and-error-states)
- [User-Friendly Error Messages](#user-friendly-error-messages)
- [Recovery Strategies](#recovery-strategies)
- [Common Patterns](#common-patterns)
- [Anti-Patterns](#anti-patterns)

## Overview

Proper error handling is essential for a good user experience.

### Key Principles
- Catch and handle all errors gracefully
- Display user-friendly error messages
- Provide recovery mechanisms
- Log errors for debugging
- Never show raw error stack traces to users

## Error Boundaries

### Basic Error Boundary
```tsx
// src/lib/components/error-boundary.tsx
import { Component, type ReactNode } from 'react'

interface Props {
  children: ReactNode
  fallback?: ReactNode
}

interface State {
  hasError: boolean
  error?: Error
}

export class ErrorBoundary extends Component<Props, State> {
  state: State = { hasError: false }

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error }
  }

  componentDidCatch(error: Error, errorInfo: React.ErrorInfo) {
    console.error('Error caught by boundary:', error, errorInfo)
  }

  render() {
    if (this.state.hasError) {
      return this.props.fallback || (
        <div className="p-4 border border-red-500 rounded bg-red-50">
          <h2 className="text-lg font-bold text-red-700">Something went wrong</h2>
          <p className="text-red-600 mt-2">
            {this.state.error?.message || 'An unexpected error occurred'}
          </p>
          <button
            onClick={() => this.setState({ hasError: false })}
            className="mt-4 px-4 py-2 bg-red-600 text-white rounded hover:bg-red-700"
          >
            Try Again
          </button>
        </div>
      )
    }

    return this.props.children
  }
}
```

### Using Error Boundaries
```tsx
// Wrap entire app
function App() {
  return (
    <ErrorBoundary>
      <RouterProvider />
    </ErrorBoundary>
  )
}

// Wrap specific features
function TasksPage() {
  return (
    <div>
      <h1>Tasks</h1>
      <ErrorBoundary fallback={<TasksErrorFallback />}>
        <TaskList />
      </ErrorBoundary>
    </div>
  )
}

// Custom fallback
function TasksErrorFallback() {
  return (
    <div className="text-center py-8">
      <p className="text-gray-600">Failed to load tasks</p>
      <button onClick={() => window.location.reload()}>
        Reload Page
      </button>
    </div>
  )
}
```

### Nested Error Boundaries
```tsx
function App() {
  return (
    <ErrorBoundary fallback={<GlobalError />}>
      <Layout>
        <ErrorBoundary fallback={<PageError />}>
          <Routes />
        </ErrorBoundary>
      </Layout>
    </ErrorBoundary>
  )
}
```

## API Error Handling

### ApiError Class
```tsx
// src/lib/api/client.ts
export class ApiError extends Error {
  statusCode: number
  response?: unknown

  constructor(message: string, statusCode: number, response?: unknown) {
    super(message)
    this.name = 'ApiError'
    this.statusCode = statusCode
    this.response = response
  }
}
```

### useApiError Hook
```tsx
// src/lib/hooks/use-api-error.ts
import { ApiError } from '@/lib/api/client'
import { toast } from '@/lib/utils/toast'

export function useApiError() {
  const showError = (error: unknown) => {
    if (error instanceof ApiError) {
      switch (error.statusCode) {
        case 400:
          toast.error('Invalid request. Please check your input.')
          break
        case 401:
          toast.error('You are not authenticated. Please log in.')
          break
        case 403:
          toast.error('You do not have permission to perform this action.')
          break
        case 404:
          toast.error('The requested resource was not found.')
          break
        case 500:
          toast.error('Server error. Please try again later.')
          break
        default:
          toast.error(error.message || 'An unexpected error occurred')
      }
    } else if (error instanceof Error) {
      toast.error(error.message)
    } else {
      toast.error('An unexpected error occurred')
    }
  }

  return { showError }
}
```

### Using in Mutations
```tsx
import { useApiError } from '@/lib/hooks/use-api-error'

export function CreateTaskForm() {
  const createTask = useCreateTaskMutation()
  const { showError } = useApiError()

  const handleSubmit = async (data: CreateTaskRequest) => {
    try {
      await createTask.mutateAsync(data)
      toast.success('Task created successfully!')
    } catch (error) {
      showError(error)
    }
  }

  return <form onSubmit={handleSubmit}>{/* ... */}</form>
}
```

## Loading and Error States

### Query States Pattern
```tsx
function TaskDetail({ taskId }: { taskId: string }) {
  const { data: task, isLoading, error, refetch } = useTaskQuery(taskId)

  if (isLoading) {
    return <LoadingSpinner />
  }

  if (error) {
    return (
      <ErrorDisplay
        error={error}
        retry={refetch}
        message="Failed to load task"
      />
    )
  }

  if (!task) {
    return <EmptyState message="Task not found" />
  }

  return <div>{/* Render task */}</div>
}
```

### ErrorDisplay Component
```tsx
import { ApiError } from '@/lib/api/client'

interface ErrorDisplayProps {
  error: unknown
  retry?: () => void
  message?: string
}

export function ErrorDisplay({ error, retry, message }: ErrorDisplayProps) {
  const errorMessage = error instanceof ApiError
    ? error.message
    : error instanceof Error
    ? error.message
    : message || 'An unexpected error occurred'

  return (
    <div className="border border-red-500 bg-red-50 rounded-lg p-4">
      <div className="flex items-start gap-3">
        <AlertCircle className="w-5 h-5 text-red-600 mt-0.5" />
        <div className="flex-1">
          <h3 className="font-medium text-red-900">Error</h3>
          <p className="text-sm text-red-700 mt-1">{errorMessage}</p>
        </div>
      </div>

      {retry && (
        <button
          onClick={retry}
          className="mt-3 px-4 py-2 text-sm bg-red-600 text-white rounded hover:bg-red-700"
        >
          Try Again
        </button>
      )}
    </div>
  )
}
```

### EmptyState Component
```tsx
interface EmptyStateProps {
  message: string
  action?: {
    label: string
    onClick: () => void
  }
}

export function EmptyState({ message, action }: EmptyStateProps) {
  return (
    <div className="flex flex-col items-center justify-center py-12 text-center">
      <div className="w-16 h-16 bg-gray-100 rounded-full flex items-center justify-center mb-4">
        <Inbox className="w-8 h-8 text-gray-400" />
      </div>
      <p className="text-gray-600 mb-4">{message}</p>
      {action && (
        <button
          onClick={action.onClick}
          className="px-4 py-2 bg-blue-600 text-white rounded hover:bg-blue-700"
        >
          {action.label}
        </button>
      )}
    </div>
  )
}
```

## User-Friendly Error Messages

### Error Message Mapping
```tsx
const ERROR_MESSAGES: Record<string, string> = {
  'network_error': 'Unable to connect. Please check your internet connection.',
  'timeout': 'Request timed out. Please try again.',
  'validation_error': 'Please check your input and try again.',
  'not_found': 'The requested item could not be found.',
  'unauthorized': 'You need to log in to continue.',
  'forbidden': 'You do not have permission to do that.',
  'conflict': 'This item already exists.',
  'server_error': 'Something went wrong on our end. Please try again later.'
}

export function getUserFriendlyMessage(error: unknown): string {
  if (error instanceof ApiError) {
    const code = error.response?.code
    return ERROR_MESSAGES[code] || error.message
  }

  return ERROR_MESSAGES.server_error
}
```

### Toast Notifications
```tsx
import { toast } from '@/lib/utils/toast'

// Success
toast.success('Task created successfully!')

// Error
toast.error('Failed to create task')

// Warning
toast.warning('Task name is too long')

// Info
toast.info('Task saved as draft')
```

## Recovery Strategies

### Retry Logic
```tsx
function TaskList() {
  const { data, error, refetch, isRefetching } = useTasksQuery()

  if (error) {
    return (
      <div>
        <ErrorDisplay error={error} />
        <button
          onClick={() => refetch()}
          disabled={isRefetching}
          className="mt-4 px-4 py-2 bg-blue-600 text-white rounded"
        >
          {isRefetching ? 'Retrying...' : 'Retry'}
        </button>
      </div>
    )
  }

  return <div>{/* Render list */}</div>
}
```

### Fallback Content
```tsx
function TaskList() {
  const { data: tasks, error } = useTasksQuery()
  const [showFallback, setShowFallback] = useState(false)

  useEffect(() => {
    if (error) {
      // Show fallback after 2 seconds
      const timer = setTimeout(() => setShowFallback(true), 2000)
      return () => clearTimeout(timer)
    }
  }, [error])

  if (error && showFallback) {
    return <TaskListSkeleton />  // Show skeleton as fallback
  }

  return <div>{tasks.map(...)}</div>
}
```

### Optimistic UI with Rollback
```tsx
export function useUpdateTaskMutation(taskId: string) {
  const queryClient = useQueryClient()
  const { showError } = useApiError()

  return useMutation({
    mutationFn: updateTask,
    onMutate: async (updates) => {
      // Optimistic update
      await queryClient.cancelQueries({ queryKey: queryKeys.tasks.detail(taskId) })
      const previous = queryClient.getQueryData(queryKeys.tasks.detail(taskId))

      queryClient.setQueryData(
        queryKeys.tasks.detail(taskId),
        (old: Task) => ({ ...old, ...updates })
      )

      return { previous }
    },
    onError: (error, _, context) => {
      // Rollback on error
      if (context?.previous) {
        queryClient.setQueryData(
          queryKeys.tasks.detail(taskId),
          context.previous
        )
      }
      showError(error)
    }
  })
}
```

## Common Patterns

### Form Validation Errors
```tsx
interface FormErrors {
  title?: string
  description?: string
}

export function TaskForm() {
  const [errors, setErrors] = useState<FormErrors>({})

  const validate = (data: FormData): FormErrors => {
    const errors: FormErrors = {}

    if (!data.title) {
      errors.title = 'Title is required'
    } else if (data.title.length < 3) {
      errors.title = 'Title must be at least 3 characters'
    }

    return errors
  }

  const handleSubmit = (data: FormData) => {
    const validationErrors = validate(data)

    if (Object.keys(validationErrors).length > 0) {
      setErrors(validationErrors)
      return
    }

    // Submit form
  }

  return (
    <form>
      <div>
        <input name="title" />
        {errors.title && (
          <p className="text-sm text-red-600 mt-1">{errors.title}</p>
        )}
      </div>
    </form>
  )
}
```

### Network Status Indicator
```tsx
export function NetworkStatus() {
  const [isOnline, setIsOnline] = useState(navigator.onLine)

  useEffect(() => {
    const handleOnline = () => setIsOnline(true)
    const handleOffline = () => setIsOnline(false)

    window.addEventListener('online', handleOnline)
    window.addEventListener('offline', handleOffline)

    return () => {
      window.removeEventListener('online', handleOnline)
      window.removeEventListener('offline', handleOffline)
    }
  }, [])

  if (!isOnline) {
    return (
      <div className="bg-yellow-100 border-b border-yellow-300 px-4 py-2 text-center">
        <p className="text-sm text-yellow-800">
          You are offline. Some features may not work.
        </p>
      </div>
    )
  }

  return null
}
```

## Anti-Patterns

### ❌ Not Handling Errors
```tsx
// WRONG
function TaskList() {
  const { data } = useTasksQuery()
  return <div>{data.map(...)}</div>  // Crashes if error or loading!
}

// CORRECT
function TaskList() {
  const { data, isLoading, error } = useTasksQuery()

  if (isLoading) return <Loading />
  if (error) return <ErrorDisplay error={error} />

  return <div>{data.map(...)}</div>
}
```

### ❌ Showing Technical Errors to Users
```tsx
// WRONG
<p>{error.stack}</p>  // Never show stack traces!

// CORRECT
<p>Something went wrong. Please try again.</p>
```

### ❌ Silent Failures
```tsx
// WRONG
try {
  await createTask(data)
} catch (error) {
  console.error(error)  // Only logs, user sees nothing!
}

// CORRECT
try {
  await createTask(data)
  toast.success('Task created!')
} catch (error) {
  showError(error)  // Show to user
  console.error(error)  // Also log for debugging
}
```

### ❌ No Recovery Options
```tsx
// WRONG
if (error) return <div>Error occurred</div>  // No way to recover

// CORRECT
if (error) {
  return (
    <ErrorDisplay
      error={error}
      retry={() => refetch()}  // Give user a way to retry
    />
  )
}
```
