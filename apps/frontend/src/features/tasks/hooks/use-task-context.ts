import type { Task } from '@moatless/types'
import { useMemo } from 'react'
import { useTaskId, useTaskStreamingState } from '../contexts/task-data-provider'
import {
  useTaskDetailQuery,
  useTaskGitDiffQuery,
  useTaskWorkspaceStatusQuery,
} from './use-task-queries'

/**
 * @deprecated Prefer dedicated query hooks (`useTaskDetailQuery`, `useTaskWorkspaceStatusQuery`, etc.)
 * in consuming components. This helper will be removed once all consumers migrate.
 */
export function useTaskData() {
  const taskId = useTaskId()
  const { streamingEvents, isStreaming, streamingError } = useTaskStreamingState()

  const taskQuery = useTaskDetailQuery(taskId)
  const gitDiffQuery = useTaskGitDiffQuery(taskId)
  const workspaceQuery = useTaskWorkspaceStatusQuery(taskId, {
    enabled: !!taskQuery.data && taskQuery.data.status === 'open',
  })

  const task = taskQuery.data as Task | undefined
  const iterations = task?.iterations ?? []
  const status = task?.status

  const canExecute = status === 'open'
  const isComplete = status === 'closed'
  const isAgentRunning = status === 'running'

  return {
    taskId,
    status,
    agentType: task?.agentType,
    repositoryId: task?.repositoryId,
    iterations,
    createdAt: task?.createdAt,
    vibe: task?.vibe,
    templateId: task?.templateId,
    name: task?.name,
    description: task?.description,
    gitDiff: gitDiffQuery.data,
    workspaceStatus: workspaceQuery.data,
    workspaceLoading: workspaceQuery.isLoading,
    canExecute,
    isAgentRunning,
    isComplete,
    streamingEvents,
    isStreaming,
    streamingError,
    isLoading: taskQuery.isLoading,
    error: taskQuery.error instanceof Error ? taskQuery.error : null,
  }
}

export function useTaskStatus() {
  const taskId = useTaskId()
  const taskQuery = useTaskDetailQuery(taskId)
  const status = taskQuery.data?.status

  return {
    status,
    isAgentRunning: status === 'running',
    canExecute: status === 'open',
    isComplete: status === 'closed',
  }
}

export function useIsAgentRunning() {
  const { isAgentRunning } = useTaskStatus()
  return isAgentRunning
}

export function useGitDiff() {
  const taskId = useTaskId()
  const gitDiffQuery = useTaskGitDiffQuery(taskId)
  return gitDiffQuery.data
}

export function useGitDiffWithLoading() {
  const taskId = useTaskId()
  const gitDiffQuery = useTaskGitDiffQuery(taskId)

  return {
    gitDiff: gitDiffQuery.data,
    isLoading: gitDiffQuery.isLoading,
    error: gitDiffQuery.error,
  }
}

export function useTaskMetadata() {
  const taskId = useTaskId()
  const taskQuery = useTaskDetailQuery(taskId)
  const data = taskQuery.data

  return useMemo(
    () => ({
      taskId,
      agentType: data?.agentType,
      repositoryId: data?.repositoryId,
      createdAt: data?.createdAt,
      vibe: data?.vibe,
      templateId: data?.templateId ?? null,
      name: data?.name ?? null,
      status: data?.status ?? null,
    }),
    [
      taskId,
      data?.agentType,
      data?.repositoryId,
      data?.createdAt,
      data?.vibe,
      data?.templateId,
      data?.name,
      data?.status,
    ],
  )
}
