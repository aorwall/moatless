import type { Task } from '@moatless/types/types/task'
import type { QueryClient } from '@tanstack/react-query'
import { queryKeys } from '@/lib/api/query-keys'

/**
 * Updates a task in all cached task list queries
 * This ensures consistency across different filtered views
 */
export function updateTaskInLists(
  queryClient: QueryClient,
  taskId: string,
  updates: Partial<Task> | Task | null,
  isAdd = false,
) {
  // Get all cached task list queries - look for any query that starts with ['tasks']
  const queryCache = queryClient.getQueryCache()
  const taskListQueries = queryCache.findAll({
    queryKey: queryKeys.tasks.all,
    exact: false,
    type: 'active',
  })

  // Update the task in each cached list
  taskListQueries.forEach((query) => {
    // Only process queries that contain 'list' in their key (task list queries)
    if (query.queryKey.includes('list')) {
      const data = query.state.data as Task[] | undefined
      if (data && Array.isArray(data)) {
        if (updates === null) {
          // Remove task
          const updatedTasks = data.filter((task) => task.taskId !== taskId)
          queryClient.setQueryData(query.queryKey, updatedTasks)
        } else if (isAdd && 'taskId' in updates) {
          // Add new task to the beginning of the list
          const existingTask = data.find((task) => task.taskId === taskId)
          if (!existingTask) {
            const updatedTasks = [updates as Task, ...data]
            queryClient.setQueryData(query.queryKey, updatedTasks)
          }
        } else {
          // Update existing task
          const updatedTasks = data.map((task) =>
            task.taskId === taskId ? { ...task, ...updates } : task,
          )
          queryClient.setQueryData(query.queryKey, updatedTasks)
        }
      }
    }
  })
}

/**
 * Removes a task from all cached task list queries
 * Used when a task is deleted
 */
export function removeTaskFromLists(queryClient: QueryClient, taskId: string) {
  const queryCache = queryClient.getQueryCache()
  const taskListQueries = queryCache.findAll({
    queryKey: queryKeys.tasks.all,
    exact: false,
    type: 'active',
  })

  taskListQueries.forEach((query) => {
    // Only process queries that contain 'list' in their key (task list queries)
    if (query.queryKey.includes('list')) {
      const data = query.state.data as Task[] | undefined
      if (data && Array.isArray(data)) {
        const updatedTasks = data.filter((task) => task.taskId !== taskId)
        queryClient.setQueryData(query.queryKey, updatedTasks)
      }
    }
  })
}

/**
 * Determines which query keys to invalidate based on task status change
 */
export function getInvalidationKeys(
  taskId: string,
  oldStatus?: string,
  newStatus?: string,
): Array<readonly unknown[]> {
  const keys: Array<readonly unknown[]> = []

  // Always invalidate the specific task
  keys.push(queryKeys.tasks.detail(taskId))

  // If status changed, invalidate relevant list queries
  if (oldStatus !== newStatus && newStatus) {
    // Task status changed - invalidate lists that might be affected
    if (newStatus === 'closed') {
      // When closing, the sidebar (non-closed tasks) needs updating
      keys.push(queryKeys.tasks.list())
    } else if (newStatus === 'merged') {
      // When merging, both open and merged lists need updating
      keys.push(queryKeys.tasks.list({ status: 'open' }))
      keys.push(queryKeys.tasks.list({ status: 'merged' }))
      keys.push(queryKeys.tasks.list())
    }
  }

  return keys
}
