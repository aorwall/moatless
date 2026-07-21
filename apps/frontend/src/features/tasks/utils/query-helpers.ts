import type { ListTasksResponse, TaskResponse } from '@moatless/api-client'
import type { QueryClient } from '@tanstack/react-query'
import { queryKeys } from '@/lib/api/query-keys'
import type { Event } from '@/lib/types/events'

type TaskListFilters = {
  activeOnly?: boolean
  limit?: number
  offset?: number
  status?: string
  tag?: string
  createdBy?: string
  followedBy?: string
  repositoryId?: string
  parentTaskId?: string
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

export function taskFromEventPayload(event: Event): TaskResponse | null {
  const payload = event.payload
  if (!isRecord(payload)) {
    return null
  }

  const task = payload['task']
  if (!isRecord(task) || typeof task['taskId'] !== 'string') {
    return null
  }

  return task as unknown as TaskResponse
}

function filtersFromQueryKey(queryKey: readonly unknown[]): TaskListFilters | undefined {
  const filters = queryKey[2]
  return isRecord(filters) ? (filters as TaskListFilters) : undefined
}

function taskMatchesFilters(task: TaskResponse, filters: TaskListFilters | undefined): boolean {
  if (!filters) {
    return true
  }
  if (filters.offset && filters.offset > 0) {
    return false
  }
  if (filters.status && task.status !== filters.status) {
    return false
  }
  if (filters.activeOnly && task.status !== 'open') {
    return false
  }
  if (filters.repositoryId && task.repositoryId !== filters.repositoryId) {
    return false
  }
  if (filters.createdBy && task.createdBy !== filters.createdBy) {
    return false
  }
  if (filters.parentTaskId && task.parentTaskId !== filters.parentTaskId) {
    return false
  }
  if (filters.followedBy && task.isFollowing !== true) {
    return false
  }
  if (
    filters.tag &&
    !task.tags.some((tag) => tag.name === filters.tag || tag.tagId === filters.tag)
  ) {
    return false
  }
  return true
}

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
    const filters = filtersFromQueryKey(queryKey)
    if (updates === null) {
      // Remove task
      const updatedTasks = data.items.filter((task) => task.taskId !== taskId)
      queryClient.setQueryData(queryKey, { ...data, items: updatedTasks })
    } else if (isAdd && 'taskId' in updates) {
      // Add new task to the beginning of the list
      const existingTask = data.items.find((task) => task.taskId === taskId)
      if (!existingTask && taskMatchesFilters(updates as TaskResponse, filters)) {
        const updatedTasks = [updates as TaskResponse, ...data.items]
        queryClient.setQueryData(queryKey, { ...data, items: updatedTasks })
      }
    } else {
      // Update existing task, removing it if the patch moves it out of this
      // filtered list. Full task payloads can also add a newly matching row.
      const fullUpdate = 'taskId' in updates ? (updates as TaskResponse) : null
      let matchedExisting = false
      const updatedTasks = data.items.flatMap((task) => {
        if (task.taskId !== taskId) {
          return [task]
        }
        matchedExisting = true
        const updatedTask = { ...task, ...updates }
        return taskMatchesFilters(updatedTask, filters) ? [updatedTask] : []
      })
      if (fullUpdate && !matchedExisting && taskMatchesFilters(fullUpdate, filters)) {
        updatedTasks.unshift(fullUpdate)
      }
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
  const eventTask = taskFromEventPayload(event)
  if (eventTask) {
    queryClient.setQueryData<TaskResponse>(queryKeys.tasks.detail(taskId), eventTask)
    updateTaskInLists(queryClient, taskId, eventTask, event.subType === 'created')
    return
  }

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
    case 'sandbox':
    case 'server':
    case 'resources':
    case 'navigation':
    case 'preview':
      // These event types do not carry task-level updates
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

    // Only update list caches for status-affecting changes (not timestamp-only updates).
    if ('status' in taskUpdates) {
      updateTaskInLists(queryClient, taskId, taskUpdates)
    }
  }
}

/**
 * Applies a `task.metadata.updated` event directly to the cache instead of
 * refetching. The backend publishes the changed `name`/`description` in the
 * event payload, so we can patch the task detail + list caches in place.
 *
 * Returns `true` if the event was fully handled via a direct write. Returns
 * `false` when the payload carries no recognized fields (e.g. a future
 * `task.updated` event with a different shape), so the caller can fall back to
 * an invalidation to stay correct.
 */
export function applyTaskMetadataUpdate(queryClient: QueryClient, event: Event): boolean {
  if (!event.taskId) {
    return false
  }

  const payload = event.payload as { name?: string | null; description?: string | null } | undefined

  const updates: Partial<TaskResponse> = {}
  // Only apply fields that are actually present as strings. A `null`/absent
  // value means the field was not part of the update (the PATCH endpoint only
  // sets provided fields), so we must not overwrite the cached value with null.
  if (typeof payload?.name === 'string') {
    updates.name = payload.name
  }
  if (typeof payload?.description === 'string') {
    updates.description = payload.description
  }

  if (Object.keys(updates).length === 0) {
    return false
  }

  const taskId = event.taskId
  const currentTask = queryClient.getQueryData<TaskResponse>(queryKeys.tasks.detail(taskId))
  if (currentTask) {
    queryClient.setQueryData(queryKeys.tasks.detail(taskId), { ...currentTask, ...updates })
  }

  // Propagate the name/description change to every cached list (the sidebar
  // renders the task name), including for other clients observing the list.
  updateTaskInLists(queryClient, taskId, updates)

  return true
}
