import { AlertCircle, Plus, RotateCcw } from 'lucide-react'
import { Component, type ReactNode, Suspense, useState } from 'react'

import { CreateTaskDialog } from '@/features/tasks/components/create-task-dialog'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { Skeleton } from '@/lib/components/ui/skeleton'

// Internal task creation fallback component
function TaskCreationFallback({
  error,
  reset,
  componentName = 'Component',
  repositoryId = '0ee846ce-fabd-4b90-9ec5-def401a5d802',
}: {
  error: Error
  reset: () => void
  componentName?: string
  repositoryId?: string
}) {
  const [showCreateTask, setShowCreateTask] = useState(false)
  console.log('repositoryId', repositoryId)

  const formatErrorForTask = () => {
    const stackTrace = error.stack || 'No stack trace available'
    const errorMessage = error.message || 'Unknown error occurred'

    return `Fix this error in ${componentName}:

Error: ${errorMessage}

Stack trace:
${stackTrace}

Please investigate and fix this issue.`
  }

  return (
    <>
      <Card className="border-destructive bg-destructive/5">
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-destructive">
            <AlertCircle className="h-5 w-5" />
            {componentName} Error
          </CardTitle>
          <CardDescription>
            Something went wrong in the {componentName.toLowerCase()} component.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="text-sm text-muted-foreground">
            <strong>Error:</strong> {error.message}
          </div>

          <div className="flex flex-col sm:flex-row gap-2">
            <Button onClick={reset} variant="outline" size="sm" className="flex items-center gap-2">
              <RotateCcw className="h-4 w-4" />
              Try Again
            </Button>

            <Button
              onClick={() => setShowCreateTask(true)}
              variant="default"
              size="sm"
              className="flex items-center gap-2"
            >
              <Plus className="h-4 w-4" />
              Create Task to Fix
            </Button>
          </div>

          <details className="text-xs">
            <summary className="cursor-pointer text-muted-foreground hover:text-foreground">
              Stack trace (development only)
            </summary>
            <pre className="mt-2 whitespace-pre-wrap bg-muted p-2 rounded text-xs overflow-auto max-h-48">
              {error.stack}
            </pre>
          </details>
        </CardContent>
      </Card>

      <CreateTaskDialog
        open={showCreateTask}
        onOpenChange={setShowCreateTask}
        defaultRepositoryId={repositoryId}
        defaultMessage={formatErrorForTask()}
        defaultMode="code"
      />
    </>
  )
}

interface ErrorBoundaryProps {
  children: ReactNode
  fallback?: (error: Error, reset: () => void) => ReactNode
  componentName?: string
  onError?: (error: Error, errorInfo: { componentStack: string }) => void
  resetKeys?: Array<string | number>
  resetOnPropsChange?: boolean
  isolate?: boolean
  repositoryId?: string // For task creation fallback
  enableTaskCreation?: boolean // Enable the task creation fallback
}

interface ErrorBoundaryState {
  hasError: boolean
  error: Error | null
  resetKeys?: Array<string | number>
}

export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  constructor(props: ErrorBoundaryProps) {
    super(props)
    this.state = {
      hasError: false,
      error: null,
      resetKeys: props.resetKeys,
    }
  }

  static getDerivedStateFromError(error: Error): Partial<ErrorBoundaryState> {
    return { hasError: true, error }
  }

  static getDerivedStateFromProps(
    props: ErrorBoundaryProps,
    state: ErrorBoundaryState,
  ): Partial<ErrorBoundaryState> | null {
    // Reset error boundary when resetKeys change
    if (props.resetKeys && state.resetKeys) {
      const hasResetKeyChanged = props.resetKeys.some(
        (key, index) => key !== state.resetKeys?.[index],
      )
      if (hasResetKeyChanged) {
        return { hasError: false, error: null, resetKeys: props.resetKeys }
      }
    }

    // Reset on any props change if requested
    if (props.resetOnPropsChange && state.hasError) {
      return { hasError: false, error: null }
    }

    return null
  }

  componentDidCatch(error: Error, errorInfo: { componentStack: string }) {
    console.error(
      `Error caught by ${this.props.componentName || 'ErrorBoundary'}:`,
      error,
      errorInfo,
    )

    // Call custom error handler if provided
    this.props.onError?.(error, errorInfo)

    // In production, you might want to send this to an error tracking service
    if (typeof window !== 'undefined' && window.location.hostname !== 'localhost') {
      // Example: sendToErrorTrackingService(error, errorInfo);
    }
  }

  reset = () => {
    this.setState({ hasError: false, error: null })
  }

  render() {
    // Check if debug mode is enabled and repository ID is available
    const isDebugMode =
      typeof window !== 'undefined' && localStorage.getItem('moatlessDebugMode') === 'true'

    if (this.state.hasError && this.state.error) {
      // Isolate mode - render nothing to prevent cascading failures
      if (this.props.isolate) {
        return null
      }

      // Custom fallback
      if (this.props.fallback) {
        return this.props.fallback(this.state.error, this.reset)
      }

      // Task creation fallback if enabled
      if (isDebugMode) {
        return (
          <TaskCreationFallback
            error={this.state.error}
            reset={this.reset}
            componentName={this.props.componentName || 'Component'}
          />
        )
      }

      // Default fallback
      return (
        <div className="h-full flex items-center justify-center p-6">
          <div className="text-center space-y-3">
            <ErrorAlert
              error={this.state.error}
              title={`${this.props.componentName || 'Component'} Error`}
              className="max-w-md w-full"
            />
          </div>
        </div>
      )
    }

    return this.props.children
  }
}

// Wrapper that combines error boundary with Suspense
export function SafeBoundary({
  children,
  componentName,
  fallback,
  loadingFallback,
  ...errorBoundaryProps
}: ErrorBoundaryProps & { loadingFallback?: ReactNode }) {
  return (
    <ErrorBoundary componentName={componentName} fallback={fallback} {...errorBoundaryProps}>
      <Suspense
        fallback={
          loadingFallback || (
            <div className="h-full flex items-center justify-center p-6">
              <Skeleton className="h-32 w-full max-w-md" />
            </div>
          )
        }
      >
        {children}
      </Suspense>
    </ErrorBoundary>
  )
}
