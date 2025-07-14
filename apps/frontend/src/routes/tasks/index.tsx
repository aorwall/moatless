import { useQueryClient } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { useEffect } from 'react'
import { useGlobalEventStream } from '@/features/events'
import { TaskDashboard } from '@/features/tasks/components/dashboard'
import { queryKeys } from '@/lib/api/query-keys'

export const Route = createFileRoute('/tasks/')({
  component: TasksIndex,
})

function TasksIndex() {
  const queryClient = useQueryClient()
  const { subscribeToAll } = useGlobalEventStream()

  // Subscribe to global events to invalidate the task list when relevant events occur
  useEffect(() => {
    console.log('[TasksIndex] Setting up global subscription for dashboard')
    return subscribeToAll((event) => {
      // Invalidate task list on relevant events
      if (
        event.subType &&
        (event.subType === 'task:created' ||
          event.subType === 'task:updated' ||
          event.subType === 'task:statusChanged' ||
          event.subType === 'iteration:created' ||
          event.subType === 'iteration:updated')
      ) {
        queryClient.invalidateQueries({ queryKey: queryKeys.tasksList() })
      }
    })
  }, [subscribeToAll, queryClient])

  return <TaskDashboard />
}
