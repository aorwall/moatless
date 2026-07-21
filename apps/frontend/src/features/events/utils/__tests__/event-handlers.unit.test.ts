import type { QueryClient } from '@tanstack/react-query'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { queryKeys } from '@/lib/api/query-keys'
import type {
  AgentEvent,
  Event,
  SandboxEvent as SandboxEventType,
  TaskEvent,
} from '@/lib/types/events'
import {
  EVENT_INVALIDATION_MAP,
  getInvalidationKeys,
  invalidateMessagesForEvent,
  isStatusAffectingEvent,
  shouldInvalidateTaskList,
} from '../event-handlers'

const refreshNewestMessagesPageMock = vi.fn((_queryClient: QueryClient, _taskId: string) =>
  Promise.resolve(),
)

vi.mock('@/features/tasks/hooks/use-tasks', () => ({
  refreshNewestMessagesPage: (queryClient: QueryClient, taskId: string) =>
    refreshNewestMessagesPageMock(queryClient, taskId),
}))

describe('event-handlers', () => {
  describe('getInvalidationKeys', () => {
    it('should return correct invalidation keys for TASK event', () => {
      const event: Event = {
        eventId: 'event-1',
        type: 'task',
        subType: 'open',
        taskId: 'task-123',
        agentType: 'test',
        isError: false,
        payload: { status: 'open', previousStatus: 'created' },
      } as TaskEvent

      const keys = getInvalidationKeys(event)

      // Should only invalidate the specific task, not all tasks
      expect(keys).toHaveLength(1)
      const [key0] = keys
      expect(key0?.('task-123')).toEqual(queryKeys.tasks.detail('task-123'))
    })

    it('should return correct invalidation keys for AGENT COMPLETED event', () => {
      const event: Event = {
        eventId: 'event-2',
        type: 'agent',
        subType: 'completed',
        taskId: 'task-456',
        agentType: 'test',
        isError: false,
        payload: {},
      } as AgentEvent

      const keys = getInvalidationKeys(event)

      // Should invalidate task detail and sandbox status (so thinking indicator clears)
      expect(keys).toHaveLength(2)
      const [agentKey0, agentKey1] = keys
      expect(agentKey0?.('task-456')).toEqual(queryKeys.tasks.detail('task-456'))
      expect(agentKey1?.('task-456')).toEqual(queryKeys.tasks.sandboxStatus('task-456'))
    })

    it('should not invalidate messages key for AGENT MESSAGE event', () => {
      const event: Event = {
        eventId: 'event-3',
        type: 'agent',
        subType: 'message',
        taskId: 'task-789',
        agentType: 'test',
        isError: false,
        payload: {},
      } as AgentEvent

      const keys = getInvalidationKeys(event)
      // Messages refresh is handled via invalidateMessagesForEvent (newest-page
      // refetch) instead of full infinite-query invalidation.
      expect(keys).toHaveLength(0)
    })

    it('should not invalidate messages key for AGENT SESSION_INIT event', () => {
      const event: Event = {
        eventId: 'event-5',
        type: 'agent',
        subType: 'session_init',
        taskId: 'task-ack',
        agentType: 'test',
        isError: false,
        timestamp: '2024-01-01T00:00:00Z',
        payload: { session_id: 'sess-1', model: 'claude' },
      } as AgentEvent

      const keys = getInvalidationKeys(event)
      expect(keys).toHaveLength(0)
    })

    it('should return correct invalidation keys for SANDBOX event', () => {
      const event: Event = {
        eventId: 'event-4',
        type: 'sandbox',
        taskId: 'task-999',
        isError: false,
        payload: {},
      } as SandboxEventType

      const keys = getInvalidationKeys(event)

      expect(keys).toHaveLength(2)
      const [sandboxKey0, sandboxKey1] = keys
      expect(sandboxKey0?.('task-999')).toEqual(queryKeys.tasks.sandboxStatus('task-999'))
      expect(sandboxKey1?.('task-999')).toEqual(queryKeys.tasks.sandboxSetup('task-999'))
    })
  })

  describe('shouldInvalidateTaskList', () => {
    it('should return true for status-affecting events', () => {
      const statusEvents = [
        { type: 'task', subType: 'open' },
        { type: 'task', subType: 'closed' },
        { type: 'taskTurn', subType: 'completed' },
        { type: 'agent', subType: 'error' },
      ]

      for (const event of statusEvents) {
        expect(shouldInvalidateTaskList(event as any)).toBe(true)
      }
    })

    it('should return false for non-status-affecting events', () => {
      const nonStatusEvents = [
        { type: 'agent', subType: 'message' },
        { type: 'agent', subType: 'session_init' },
        { type: 'taskTurn', subType: 'created' },
      ]

      for (const event of nonStatusEvents) {
        expect(shouldInvalidateTaskList(event as any)).toBe(false)
      }
    })
  })

  describe('isStatusAffectingEvent', () => {
    it('should identify status-affecting events correctly', () => {
      const statusEvents = [
        { type: 'task', subType: 'open' },
        { type: 'taskTurn', subType: 'active' },
        { type: 'agent', subType: 'started' },
      ]

      for (const event of statusEvents) {
        expect(isStatusAffectingEvent(event as any)).toBe(true)
      }
    })
  })

  describe('EVENT_INVALIDATION_MAP', () => {
    it('should not include tasks.all invalidation for any event', () => {
      // Verify that no event type invalidates all tasks
      for (const typeMapping of Object.values(EVENT_INVALIDATION_MAP)) {
        const invalidationFns = Array.isArray(typeMapping)
          ? typeMapping
          : Object.values(typeMapping).flat()
        for (const fn of invalidationFns) {
          if (typeof fn === 'function') {
            const result = fn('test-task-id')
            // Ensure no invalidation key equals ['tasks'] which would invalidate all tasks
            expect(result).not.toEqual(['tasks'])
            expect(result).not.toEqual(queryKeys.tasks.all)
          }
        }
      }
    })

    it('should only invalidate specific task resources', () => {
      // Verify TASK only invalidates the specific task
      const taskInvalidations = EVENT_INVALIDATION_MAP.task['*']
      expect(taskInvalidations).toHaveLength(1)
      expect(taskInvalidations[0]('task-123')).toEqual(queryKeys.tasks.detail('task-123'))
    })

    it('should not invalidate the messages key for any event', () => {
      // Messages refresh now goes through invalidateMessagesForEvent which
      // surgically refetches the newest page; no event should invalidate
      // the entire messages infinite query.
      for (const typeMapping of Object.values(EVENT_INVALIDATION_MAP)) {
        const invalidationFns = Array.isArray(typeMapping)
          ? typeMapping
          : Object.values(typeMapping).flat()
        for (const fn of invalidationFns) {
          expect(fn).not.toBe(queryKeys.tasks.messages)
        }
      }
    })
  })

  describe('invalidateMessagesForEvent', () => {
    // `observerCount` controls whether the chat view is considered mounted for
    // the task in the event. `invalidateQueries` captures the stale-marking
    // path taken when no chat view observes the messages query.
    let observerCount = 1
    const invalidateQueriesMock = vi.fn()
    const findMock = vi.fn(() => ({ getObserversCount: () => observerCount }))
    const queryClient = {
      getQueryCache: () => ({ find: findMock }),
      invalidateQueries: invalidateQueriesMock,
    } as unknown as QueryClient

    beforeEach(() => {
      observerCount = 1
      refreshNewestMessagesPageMock.mockClear()
      invalidateQueriesMock.mockClear()
      findMock.mockClear()
    })

    const trigger = (event: Event) => invalidateMessagesForEvent(event, queryClient)

    const messageEvent = (overrides: Record<string, unknown>): Event =>
      ({
        eventId: 'e',
        taskId: 'task-1',
        agentType: 'test',
        isError: false,
        payload: {},
        ...overrides,
      }) as Event

    it.each([
      { type: 'taskTurn', subType: 'completed' },
      { type: 'taskTurn', subType: 'error' },
      { type: 'agent', subType: 'session_init' },
    ] as const)(
      'reconciles the newest page on $type/$subType when the chat view is mounted',
      ({ type, subType }) => {
        trigger(messageEvent({ type, subType }))

        expect(refreshNewestMessagesPageMock).toHaveBeenCalledTimes(1)
        expect(refreshNewestMessagesPageMock).toHaveBeenCalledWith(queryClient, 'task-1')
        expect(invalidateQueriesMock).not.toHaveBeenCalled()
      },
    )

    it('does not refetch on streaming agent.message (message.upsert keeps the Chat live)', () => {
      trigger(messageEvent({ type: 'agent', subType: 'message', taskId: 'task-stream' }))

      expect(refreshNewestMessagesPageMock).not.toHaveBeenCalled()
      expect(invalidateQueriesMock).not.toHaveBeenCalled()
    })

    it('marks the query stale without fetching when no chat view is mounted', () => {
      observerCount = 0

      trigger(messageEvent({ type: 'taskTurn', subType: 'completed', taskId: 'task-bg' }))

      expect(refreshNewestMessagesPageMock).not.toHaveBeenCalled()
      expect(invalidateQueriesMock).toHaveBeenCalledWith({
        queryKey: queryKeys.tasks.messages('task-bg'),
        refetchType: 'none',
      })
    })

    it('ignores events outside the trigger set', () => {
      trigger({
        eventId: 'e',
        type: 'agent',
        subType: 'started',
        taskId: 'task-x',
        agentType: 'test',
        isError: false,
        payload: {},
      } as AgentEvent)

      expect(refreshNewestMessagesPageMock).not.toHaveBeenCalled()
      expect(invalidateQueriesMock).not.toHaveBeenCalled()
    })

    it('ignores events with no taskId', () => {
      trigger({
        eventId: 'e',
        type: 'taskTurn',
        subType: 'completed',
        taskId: '',
        agentType: 'test',
        isError: false,
        payload: {},
      } as Event)

      expect(refreshNewestMessagesPageMock).not.toHaveBeenCalled()
      expect(invalidateQueriesMock).not.toHaveBeenCalled()
    })
  })
})
