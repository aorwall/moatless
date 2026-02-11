import type { ListTasksResponse, TaskResponse } from '@moatless/api-client'
import type { Event } from '@moatless/types'
import type { QueryClient } from '@tanstack/react-query'
import { queryKeys } from '@/lib/api/query-keys'

/**
 * Updates a task in all cached task list queries
 * This ensures consistency across different filtered views
 */
export function updateTaskInLists(
  queryClient: QueryClient,
  taskId: string,
  updates: Partial<TaskResponse> | TaskResponse | null,
  isAdd = false,
) {
  // Get all cached task list queries - look for any query that starts with ['tasks']
  const queryCache = queryClient.getQueryCache()
  const taskListQueries = queryCache.findAll({
    queryKey: queryKeys.tasks.all,
    exact: false,
    type: 'active',
  })

  // Helper: Update task list data based on operation type
  const updateTaskListData = (
    queryClient: QueryClient,
    queryKey: readonly unknown[],
    data: ListTasksResponse,
    taskId: string,
    updates: Partial<TaskResponse> | TaskResponse | null,
    isAdd: boolean,
  ): void => {
    if (updates === null) {
      // Remove task
      const updatedTasks = data.items.filter((task) => task.taskId !== taskId)
      queryClient.setQueryData(queryKey, { ...data, items: updatedTasks })
    } else if (isAdd && 'taskId' in updates) {
      // Add new task to the beginning of the list
      const existingTask = data.items.find((task) => task.taskId === taskId)
      if (!existingTask) {
        const updatedTasks = [updates as TaskResponse, ...data.items]
        queryClient.setQueryData(queryKey, { ...data, items: updatedTasks })
      }
    } else {
      // Update existing task
      const updatedTasks = data.items.map((task) =>
        task.taskId === taskId ? { ...task, ...updates } : task,
      )
      queryClient.setQueryData(queryKey, { ...data, items: updatedTasks })
    }
  }

  // Update the task in each cached list
  for (const query of taskListQueries) {
    // Only process queries that contain 'list' in their key (task list queries)
    if (!query.queryKey.includes('list')) {
      continue
    }

    const data = query.state.data as ListTasksResponse | undefined
    if (data?.items && Array.isArray(data.items)) {
      updateTaskListData(queryClient, query.queryKey, data, taskId, updates, isAdd)
    }
  }
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

  for (const query of taskListQueries) {
    // Only process queries that contain 'list' in their key (task list queries)
    if (query.queryKey.includes('list')) {
      const data = query.state.data as ListTasksResponse | undefined
      if (data?.items && Array.isArray(data.items)) {
        const updatedTasks = data.items.filter((task) => task.taskId !== taskId)
        queryClient.setQueryData(query.queryKey, {
          ...data,
          items: updatedTasks,
        })
      }
    }
  }
}

// Helper: Get task updates for task events
function getTaskEventUpdates(event: Event): Partial<TaskResponse> {
  if (event.subType === 'open') {
    return { status: 'open' }
  }
  if (event.subType === 'closed') {
    return { status: 'closed' }
  }
  return {}
}

// Helper: Get task updates for task turn events
function getTaskTurnUpdates(event: Event): Partial<TaskResponse> {
  const timestamp = new Date(event.timestamp || Date.now()).toISOString()

  if (event.subType === 'active') {
    return {} // Running state is derived from messages + envStatus
  }
  if (event.subType === 'completed' || event.subType === 'error') {
    return { status: 'open', lastMessageAt: timestamp }
  }
  return {}
}

// Helper: Get task updates for agent events
function getAgentEventUpdates(event: Event): Partial<TaskResponse> {
  const timestamp = new Date(event.timestamp || Date.now()).toISOString()

  if (event.subType === 'started') {
    return {} // Running state is derived from messages + envStatus
  }
  if (event.subType === 'stopped' || event.subType === 'completed') {
    return { status: 'open', lastMessageAt: timestamp }
  }
  if (event.subType === 'message') {
    return { lastMessageAt: timestamp }
  }
  return {}
}

/**
 * Applies optimistic updates to task data based on incoming events
 */
export function applyTaskEventUpdate(queryClient: QueryClient, event: Event): void {
  if (!event.taskId) {
    return
  }

  const taskId = event.taskId

  // Get updates based on event type
  let taskUpdates: Partial<TaskResponse> = {}

  switch (event.type) {
    case 'task':
      taskUpdates = getTaskEventUpdates(event)
      break
    case 'taskTurn':
      taskUpdates = getTaskTurnUpdates(event)
      break
    case 'agent':
      taskUpdates = getAgentEventUpdates(event)
      break
  }

  // Apply updates if we have any
  if (Object.keys(taskUpdates).length > 0) {
    // Get current task data if it exists
    const currentTask = queryClient.getQueryData<TaskResponse>(queryKeys.tasks.detail(taskId))

    // Update task detail cache only if it exists
    if (currentTask) {
      const updatedTask = { ...currentTask, ...taskUpdates }
      queryClient.setQueryData(queryKeys.tasks.detail(taskId), updatedTask)
    }

    // Always update task in all list caches, even if detail isn't cached
    // This ensures sidebar and dashboard stay in sync with events
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
