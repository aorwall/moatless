import type { WorkspaceStatusResponse } from '@moatless/shared/types/api'
import type { GitDiffResponse, TaskIteration } from '@moatless/shared/types/task'
import { useQuery } from '@tanstack/react-query'
import { createContext, type ReactNode, useContext, useMemo } from 'react'
import { queryKeys } from '@/lib/api/query-keys'
import { tasksApi } from '../api/tasks'

interface TaskDataContextValue {
  // Task metadata (rarely changes)
  taskId: string
  status: string | undefined
  agentType: string | undefined
  repositoryId: string | undefined
  iterations: TaskIteration[]
  createdAt: Date | undefined

  // Git diff data
  gitDiff: GitDiffResponse | undefined

  // Workspace status (conditional polling)
  workspaceStatus: string | undefined
  workspaceReady: boolean
  workspaceHasChanges: boolean

  // Derived flags for components (stable primitives)
  canExecute: boolean
  isRunning: boolean
  isComplete: boolean

  // Loading & error states
  isLoading: boolean
  error: Error | null
}

const TaskDataContext = createContext<TaskDataContextValue | undefined>(undefined)

interface TaskDataProviderProps {
  taskId: string
  children: ReactNode
}

/**
 * TaskDataProvider manages stable task metadata and workspace status.
 * Uses React Query without complex selectors to minimize re-renders.
 */
export function TaskDataProvider({ taskId, children }: TaskDataProviderProps) {
  // Task metadata query - simplified without select
  const taskQuery = useQuery({
    queryKey: queryKeys.task(taskId),
    queryFn: () => tasksApi.getTask(taskId),
    staleTime: 30000, // 30s - task metadata doesn't change frequently
    enabled: !!taskId && !taskId.startsWith('temp-'),
  })

  // Workspace status query - simplified without select
  const workspaceQuery = useQuery({
    queryKey: queryKeys.taskWorkspaceStatus(taskId),
    queryFn: () => tasksApi.getTaskWorkspaceStatus(taskId),
    enabled: !!taskId && !taskId.startsWith('temp-'),
    refetchInterval: (query) => {
      const data = query.state.data as WorkspaceStatusResponse | undefined
      const status = data?.status
      // Only poll when in transitional states
      if (status === 'setting_up' || status === 'running') {
        return 3000 // 3 seconds for transitional states
      }
      return false // Stop polling when stable
    },
    staleTime: 2000, // Consider data stale after 2 seconds
  })

  // Git diff query
  const gitDiffQuery = useQuery({
    queryKey: queryKeys.taskGitDiff(taskId),
    queryFn: () => tasksApi.getTaskGitDiff(taskId),
    enabled: !!taskId && !taskId.startsWith('temp-'),
    staleTime: 10 * 60 * 1000, // 10 minutes - git diff doesn't change frequently
    retry: false, // Don't retry on failure
  })

  // Extract primitive values in useMemo to ensure stability
  const taskPrimitives = useMemo(() => {
    const task = taskQuery.data
    if (!task) return null

    return {
      status: task.status,
      agentType: task.agentType,
      repositoryId: task.repositoryId,
      iterations: task.iterations || [],
      createdAt: task.createdAt,
      isRunning: task.status === 'running',
      canExecute: task.status === 'open' || task.status === 'running',
      isComplete: task.status === 'closed' || task.status === 'merged' || task.status === 'error',
    }
  }, [taskQuery.data])

  const workspacePrimitives = useMemo(() => {
    const workspace = workspaceQuery.data
    if (!workspace) return null

    return {
      status: workspace.status,
      isReady: workspace.status === 'idle' || workspace.status === 'running',
      hasChanges: workspace.hasUncommittedChanges || workspace.hasUnmergedCommits || false,
    }
  }, [workspaceQuery.data])

  // Memoized context value with stable primitive dependencies
  const contextValue: TaskDataContextValue = useMemo(
    () => ({
      // Task metadata primitives
      taskId,
      status: taskPrimitives?.status,
      agentType: taskPrimitives?.agentType,
      repositoryId: taskPrimitives?.repositoryId,
      iterations: taskPrimitives?.iterations || [],
      createdAt: taskPrimitives?.createdAt,

      // Git diff data
      gitDiff: gitDiffQuery.data,

      // Workspace primitives
      workspaceStatus: workspacePrimitives?.status,
      workspaceReady: workspacePrimitives?.isReady ?? false,
      workspaceHasChanges: workspacePrimitives?.hasChanges ?? false,

      // Derived flags (stable primitives)
      canExecute: taskPrimitives?.canExecute ?? false,
      isRunning: taskPrimitives?.isRunning ?? false,
      isComplete: taskPrimitives?.isComplete ?? false,

      // Loading & error states
      isLoading: taskQuery.isLoading || workspaceQuery.isLoading || gitDiffQuery.isLoading,
      error: taskQuery.error || workspaceQuery.error || gitDiffQuery.error || null,
    }),
    [
      taskId,
      taskPrimitives,
      workspacePrimitives,
      gitDiffQuery.data,
      taskQuery.isLoading,
      workspaceQuery.isLoading,
      gitDiffQuery.isLoading,
      taskQuery.error,
      workspaceQuery.error,
      gitDiffQuery.error,
    ],
  )

  return <TaskDataContext.Provider value={contextValue}>{children}</TaskDataContext.Provider>
}

/**
 * Hook to access stable task data and status.
 * Returns primitives that only change when actual values change.
 */
export function useTaskData() {
  const context = useContext(TaskDataContext)
  if (!context) {
    throw new Error('useTaskData must be used within TaskDataProvider')
  }
  return context
}

// Specialized hooks for specific data slices
export function useTaskStatus() {
  const { status, isRunning, canExecute, isComplete } = useTaskData()
  return { status, isRunning, canExecute, isComplete }
}

export function useWorkspaceStatus() {
  const { workspaceStatus, workspaceReady, workspaceHasChanges } = useTaskData()
  return { workspaceStatus, workspaceReady, workspaceHasChanges }
}

export function useTaskIterations() {
  const { iterations } = useTaskData()
  return iterations
}

export function useGitDiff() {
  const { gitDiff } = useTaskData()
  return gitDiff
}

// Performance optimized hooks that only return specific fields
export function useTaskLoadingError() {
  const { isLoading, error } = useTaskData()
  return { isLoading, error }
}

export function useTaskMetadata() {
  const { taskId, agentType, repositoryId, createdAt } = useTaskData()
  return { taskId, agentType, repositoryId, createdAt }
}
