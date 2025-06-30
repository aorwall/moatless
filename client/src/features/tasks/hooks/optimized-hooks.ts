/**
 * Optimized hooks for performance-critical components
 * These hooks provide granular subscriptions to specific task data
 * to minimize unnecessary re-renders
 */

import { useTaskData } from '../contexts/task-providers'

/**
 * Hook that only subscribes to task streaming status
 * Used by components that only need to know if task is running
 */
export function useStreamingStatus(_taskId: string) {
  const { status, iterations } = useTaskData()

  // Calculate if task is running
  const isRunning = status === 'running'
  const canExecute = status === 'ready' || status === 'error' || status === 'completed'
  const isComplete = status === 'completed'

  // Get last iteration status if needed
  const lastIteration =
    iterations && iterations.length > 0 ? iterations[iterations.length - 1] : null
  const hasError = lastIteration?.status === 'error' || lastIteration?.status === 'failed'

  return {
    status,
    isRunning,
    canExecute,
    isComplete,
    hasError,
  }
}

/**
 * Hook that only subscribes to task loading and error states
 * Used by components that display loading/error UI but don't need task data
 */
export function useTaskLoadingState() {
  const { isLoading, error } = useTaskData()

  return {
    isLoading,
    error,
  }
}

/**
 * Hook that only subscribes to task status changes
 * Used by TaskHeader and similar components
 */
export function useTaskStatusOnly() {
  const { status, iterations } = useTaskData()

  const isRunning = status === 'running'
  const lastIteration =
    iterations && iterations.length > 0 ? iterations[iterations.length - 1] : null

  return {
    status,
    isRunning,
    iterationCount: iterations?.length || 0,
    lastIterationStatus: lastIteration?.status,
  }
}
