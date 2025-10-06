import type { Event, Task, TaskListResponse } from '@moatless/types'
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
      const data = query.state.data as TaskListResponse | undefined
      if (data?.items && Array.isArray(data.items)) {
        if (updates === null) {
          // Remove task
          const updatedTasks = data.items.filter((task) => task.taskId !== taskId)
          queryClient.setQueryData(query.queryKey, {
            ...data,
            items: updatedTasks,
          })
        } else if (isAdd && 'taskId' in updates) {
          // Add new task to the beginning of the list
          const existingTask = data.items.find((task) => task.taskId === taskId)
          if (!existingTask) {
            const updatedTasks = [updates as Task, ...data.items]
            queryClient.setQueryData(query.queryKey, {
              ...data,
              items: updatedTasks,
            })
          }
        } else {
          // Update existing task
          const updatedTasks = data.items.map((task) =>
            task.taskId === taskId ? { ...task, ...updates } : task,
          )
          queryClient.setQueryData(query.queryKey, {
            ...data,
            items: updatedTasks,
          })
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
      const data = query.state.data as TaskListResponse | undefined
      if (data?.items && Array.isArray(data.items)) {
        const updatedTasks = data.items.filter((task) => task.taskId !== taskId)
        queryClient.setQueryData(query.queryKey, {
          ...data,
          items: updatedTasks,
        })
      }
    }
  })
}

/**
 * Applies optimistic updates to task data based on incoming events
 */
export function applyTaskEventUpdate(queryClient: QueryClient, event: Event): void {
  if (!event.taskId) {
    return
  }

  const taskId = event.taskId

  // Get current task data
  const currentTask = queryClient.getQueryData<Task>(queryKeys.tasks.detail(taskId))
  if (!currentTask) {
    return
  }

  let taskUpdates: Partial<Task> = {}

  // Handle different event types
  switch (event.type) {
    case 'task':
      if (event.subType === 'open') {
        taskUpdates = { status: 'open' }
      } else if (event.subType === 'closed') {
        taskUpdates = { status: 'closed' }
      }
      break

    case 'taskIteration':
      if (event.subType === 'active') {
        // Task iteration is now active - task is running
        taskUpdates = { status: 'running' }
      } else if (event.subType === 'completed') {
        // Iteration completed - task is back to open
        taskUpdates = {
          status: 'open',
          lastMessageAt: new Date(event.timestamp || Date.now()),
        }
      } else if (event.subType === 'error') {
        // Iteration errored - task is back to open
        taskUpdates = {
          status: 'open',
          lastMessageAt: new Date(event.timestamp || Date.now()),
        }
      }
      break

    case 'agent':
      if (event.subType === 'started') {
        // Agent started - task is running
        taskUpdates = { status: 'running' }
      } else if (event.subType === 'stopped') {
        // Agent stopped - task is back to open
        taskUpdates = {
          status: 'open',
          lastMessageAt: new Date(event.timestamp || Date.now()),
        }
      } else if (event.subType === 'completed') {
        // Agent completed - task is back to open
        taskUpdates = {
          status: 'open',
          lastMessageAt: new Date(event.timestamp || Date.now()),
        }
      } else if (event.subType === 'message') {
        // New message - update last message time
        taskUpdates = {
          lastMessageAt: new Date(event.timestamp || Date.now()),
        }
      }
      break
  }

  // Apply updates if we have any
  if (Object.keys(taskUpdates).length > 0) {
    const updatedTask = { ...currentTask, ...taskUpdates }

    // Update task detail cache
    queryClient.setQueryData(queryKeys.tasks.detail(taskId), updatedTask)

    // Update task in all list caches
    updateTaskInLists(queryClient, taskId, taskUpdates)
  }
}

/**
 * Determines which query keys to invalidate based on task status change
 */
export function getInvalidationKeys(
  taskId: string,
  oldStatus?: string,
  newStatus?: string,
): (readonly unknown[])[] {
  const keys: (readonly unknown[])[] = []

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
