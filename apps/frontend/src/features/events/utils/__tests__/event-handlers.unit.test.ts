import type {
  AgentEvent,
  Event,
  SandboxEvent as SandboxEventType,
  TaskEvent,
} from '@moatless/types'
import { describe, expect, it } from 'vitest'
import { queryKeys } from '@/lib/api/query-keys'
import {
  EVENT_INVALIDATION_MAP,
  getInvalidationKeys,
  isStatusAffectingEvent,
  shouldInvalidateTaskList,
} from '../event-handlers'

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
      expect(keys[0]('task-123')).toEqual(queryKeys.tasks.detail('task-123'))
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

      // Should invalidate task detail
      expect(keys).toHaveLength(1)
      expect(keys[0]('task-456')).toEqual(queryKeys.tasks.detail('task-456'))
    })

    it('should return correct invalidation keys for AGENT MESSAGE event', () => {
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
      expect(keys).toHaveLength(1)
      expect(keys[0]).toBe(queryKeys.tasks.messages)
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
      expect(keys[0]('task-999')).toEqual(queryKeys.tasks.sandboxStatus('task-999'))
      expect(keys[1]('task-999')).toEqual(queryKeys.tasks.sandboxSetup('task-999'))
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
  })
})
