import type { TaskResponse } from '@moatless/api-client'
import { describe, expect, it } from 'vitest'
import { buildTaskGroups, type EnrichedTask } from '../sidebar-task-groups'

const BASE_TIME = '2026-01-01T00:00:00.000Z'

function task(taskId: string, parentTaskId?: string): TaskResponse {
  return {
    agentType: 'claude-code',
    bindings: [],
    createdAt: BASE_TIME,
    inactiveTtlDays: 7,
    lastMessageAt: BASE_TIME,
    name: taskId,
    ...(parentTaskId !== undefined && { parentTaskId }),
    relations: [],
    repositoryId: 'repo-1',
    repositoryName: 'Repo',
    sandboxDesiredState: 'running',
    sandboxDesiredStateUpdatedAt: BASE_TIME,
    scope: 'user',
    status: 'open',
    tags: [],
    taskId,
    title: taskId,
    totalCostUsd: 0,
    ttlMinutes: 60,
    visibility: 'private',
  }
}

function enriched(taskId: string, parentTaskId?: string): EnrichedTask {
  return {
    task: task(taskId, parentTaskId),
    repositoryName: 'Repo',
    state: 'idle',
    unread: false,
    preview: null,
    recentMessages: [],
    lastActivityAt: BASE_TIME,
  }
}

describe('buildTaskGroups', () => {
  it('groups visible direct children under their parent', () => {
    const groups = buildTaskGroups([enriched('parent'), enriched('child', 'parent')])

    expect(groups).toHaveLength(1)
    expect(groups[0]?.main.task.taskId).toBe('parent')
    expect(groups[0]?.children).toHaveLength(1)
    expect(groups[0]?.children[0]?.main.task.taskId).toBe('child')
  })

  it('keeps a child top-level when its parent is not visible', () => {
    const groups = buildTaskGroups([enriched('child', 'missing-parent')])

    expect(groups).toHaveLength(1)
    expect(groups[0]?.main.task.taskId).toBe('child')
    expect(groups[0]?.children).toHaveLength(0)
  })

  it('preserves visible multi-level parent chains', () => {
    const groups = buildTaskGroups([
      enriched('parent'),
      enriched('child', 'parent'),
      enriched('grandchild', 'child'),
    ])

    expect(groups).toHaveLength(1)
    expect(groups[0]?.main.task.taskId).toBe('parent')
    expect(groups[0]?.children[0]?.main.task.taskId).toBe('child')
    expect(groups[0]?.children[0]?.children[0]?.main.task.taskId).toBe('grandchild')
  })
})
