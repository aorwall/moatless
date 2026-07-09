import type { TaskResponse } from '@moatless/api-client'
import { QueryClient } from '@tanstack/react-query'
import { beforeEach, describe, expect, it } from 'vitest'
import { queryKeys } from '@/lib/api/query-keys'
import type { Event } from '@/lib/types/events'
import { applyTaskMetadataUpdate } from '../query-helpers'

const baseTask = {
  taskId: 'task-1',
  name: 'Old name',
  description: 'Old description',
  status: 'open',
} as unknown as TaskResponse

const metadataEvent = (payload: unknown, taskId = 'task-1'): Event =>
  ({
    eventId: 'e1',
    type: 'task',
    subType: 'metadata.updated',
    taskId,
    agentType: 'test',
    isError: false,
    payload,
  }) as unknown as Event

describe('applyTaskMetadataUpdate', () => {
  let queryClient: QueryClient

  beforeEach(() => {
    queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    })
    queryClient.setQueryData(queryKeys.tasks.detail('task-1'), baseTask)
  })

  it('patches name and description into the detail cache and returns true', () => {
    const handled = applyTaskMetadataUpdate(
      queryClient,
      metadataEvent({ name: 'New name', description: 'New description' }),
    )

    expect(handled).toBe(true)
    expect(queryClient.getQueryData<TaskResponse>(queryKeys.tasks.detail('task-1'))).toMatchObject({
      name: 'New name',
      description: 'New description',
      status: 'open',
    })
  })

  it('only patches the fields present in the payload', () => {
    const handled = applyTaskMetadataUpdate(queryClient, metadataEvent({ name: 'Only name' }))

    expect(handled).toBe(true)
    expect(queryClient.getQueryData<TaskResponse>(queryKeys.tasks.detail('task-1'))).toMatchObject({
      name: 'Only name',
      description: 'Old description',
    })
  })

  it('does not overwrite cached values when the payload field is null', () => {
    const handled = applyTaskMetadataUpdate(
      queryClient,
      metadataEvent({ name: 'New name', description: null }),
    )

    expect(handled).toBe(true)
    expect(queryClient.getQueryData<TaskResponse>(queryKeys.tasks.detail('task-1'))).toMatchObject({
      name: 'New name',
      description: 'Old description',
    })
  })

  it('returns false when the payload carries no recognized fields', () => {
    const handled = applyTaskMetadataUpdate(queryClient, metadataEvent({ message: 'noise' }))

    expect(handled).toBe(false)
    // Cache is untouched so the caller can fall back to a refetch.
    expect(queryClient.getQueryData<TaskResponse>(queryKeys.tasks.detail('task-1'))).toMatchObject({
      name: 'Old name',
      description: 'Old description',
    })
  })

  it('returns false when the event has no taskId', () => {
    const handled = applyTaskMetadataUpdate(queryClient, metadataEvent({ name: 'New name' }, ''))

    expect(handled).toBe(false)
  })

  it('returns true even when the detail cache is empty (list-only update)', () => {
    queryClient.removeQueries({ queryKey: queryKeys.tasks.detail('task-1') })

    const handled = applyTaskMetadataUpdate(queryClient, metadataEvent({ name: 'New name' }))

    expect(handled).toBe(true)
    expect(queryClient.getQueryData(queryKeys.tasks.detail('task-1'))).toBeUndefined()
  })
})
