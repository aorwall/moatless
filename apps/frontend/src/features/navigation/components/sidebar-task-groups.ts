import type { TaskResponse } from '@moatless/api-client'
import type { TaskActivityState } from '@/features/tasks/utils/task-status'

export interface EnrichedTask {
  task: TaskResponse
  repositoryName: string
  state: TaskActivityState
  unread: boolean
  preview: string | null
  recentMessages: string[]
  lastActivityAt: string
}

export interface TaskGroup {
  main: EnrichedTask
  children: TaskGroup[]
  state: TaskActivityState
  lastActivityAt: string
}

export const RUNNING_STATE_ORDER: Record<TaskActivityState, number> = {
  'agent-working': 0,
  starting: 1,
  'sandbox-running': 2,
  idle: 3,
}

export function compareActivityDesc(a: EnrichedTask, b: EnrichedTask) {
  return b.lastActivityAt.localeCompare(a.lastActivityAt)
}

function mostActiveState(tasks: EnrichedTask[]): TaskActivityState {
  return tasks.reduce<TaskActivityState>((best, item) => {
    return RUNNING_STATE_ORDER[item.state] < RUNNING_STATE_ORDER[best] ? item.state : best
  }, 'idle')
}

function latestActivityAt(tasks: EnrichedTask[]): string {
  return tasks.reduce((latest, item) => {
    return item.lastActivityAt.localeCompare(latest) > 0 ? item.lastActivityAt : latest
  }, tasks[0]?.lastActivityAt ?? '')
}

export function flattenGroupTasks(group: TaskGroup): EnrichedTask[] {
  return [group.main, ...group.children.flatMap(flattenGroupTasks)]
}

export function visibleGroupTaskIds(group: TaskGroup, expandedGroupIds: Set<string>): string[] {
  if (!expandedGroupIds.has(group.main.task.taskId)) {
    return [group.main.task.taskId]
  }
  return [
    group.main.task.taskId,
    ...group.children.flatMap((child) => visibleGroupTaskIds(child, expandedGroupIds)),
  ]
}

export function buildTaskGroups(items: EnrichedTask[]): TaskGroup[] {
  const byId = new Map(items.map((item) => [item.task.taskId, item]))
  const childrenByParent = new Map<string, EnrichedTask[]>()

  for (const item of items) {
    const parentId = item.task.parentTaskId
    if (!(parentId && byId.has(parentId))) {
      continue
    }

    const siblings = childrenByParent.get(parentId) ?? []
    siblings.push(item)
    childrenByParent.set(parentId, siblings)
  }

  const buildGroup = (main: EnrichedTask, ancestors: Set<string>): TaskGroup => {
    const nextAncestors = new Set(ancestors)
    nextAncestors.add(main.task.taskId)
    const children = (childrenByParent.get(main.task.taskId) ?? [])
      .filter((child) => !nextAncestors.has(child.task.taskId))
      .sort(compareActivityDesc)
      .map((child) => buildGroup(child, nextAncestors))
    const group: TaskGroup = {
      main,
      children,
      state: 'idle',
      lastActivityAt: main.lastActivityAt,
    }
    const family = flattenGroupTasks(group)
    return {
      ...group,
      state: mostActiveState(family),
      lastActivityAt: latestActivityAt(family),
    }
  }

  const roots = items.filter((item) => {
    const parentId = item.task.parentTaskId
    return !(parentId && byId.has(parentId))
  })

  const visibleRoots = roots.length > 0 ? roots : items
  return visibleRoots.map((item) => buildGroup(item, new Set()))
}
