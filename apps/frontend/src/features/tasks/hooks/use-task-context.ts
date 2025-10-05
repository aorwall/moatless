import { useMemo } from 'react'
import { useTaskId } from '../contexts/task-data-provider'
import { useTaskDetailQuery, useTaskGitDiffQuery } from './use-task-queries'

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
