import { useMemo } from 'react'
import { useTaskId } from '@/features/tasks/contexts/task-data-provider'
import { useTaskDetailQuery } from './use-task-queries'

export function useTaskStatus() {
  const taskId = useTaskId()
  const taskQuery = useTaskDetailQuery(taskId)
  const status = taskQuery.data?.status

  return {
    status,
    // Agent running state is tracked via workspace agentState, not task status
    isAgentRunning: false,
    canExecute: status === 'open',
    isComplete: status === 'closed',
  }
}

export function useIsAgentRunning() {
  const { isAgentRunning } = useTaskStatus()
  return isAgentRunning
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
