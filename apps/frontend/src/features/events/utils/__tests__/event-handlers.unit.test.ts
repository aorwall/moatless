import type { Event } from '@moatless/types'
import { describe, expect, it } from 'vitest'
import { queryKeys } from '@/lib/api/query-keys'
import {
  EVENT_INVALIDATION_MAP,
  getInvalidationKeys,
  isStatusAffectingEvent,
  STATUS_AFFECTING_EVENTS,
  shouldInvalidateTaskList,
} from '../event-handlers'

describe('event-handlers', () => {
  describe('getInvalidationKeys', () => {
    it('should return correct invalidation keys for TASK_STATUS event', () => {
      const event: Event = {
        eventId: 'event-1',
        type: 'task_status',
        taskId: 'task-123',
        agentType: 'test',
        isError: false,
        payload: { status: 'running' },
      }

      const keys = getInvalidationKeys(event)

      // Should only invalidate the specific task, not all tasks
      expect(keys).toHaveLength(1)
      expect(keys[0]('task-123')).toEqual(queryKeys.tasks.detail('task-123'))
    })

    it('should return correct invalidation keys for RESULT event', () => {
      const event: Event = {
        eventId: 'event-2',
        type: 'result',
        taskId: 'task-456',
        agentType: 'test',
        isError: false,
        payload: {},
      }

      const keys = getInvalidationKeys(event)

      // Should invalidate task detail and git diff
      expect(keys).toHaveLength(2)
      expect(keys[0]('task-456')).toEqual(queryKeys.tasks.detail('task-456'))
      expect(keys[1]('task-456')).toEqual(queryKeys.tasks.gitDiff('task-456'))
    })

    it('should return correct invalidation keys for MESSAGE event', () => {
      const event: Event = {
        eventId: 'event-3',
        type: 'message',
        taskId: 'task-789',
        agentType: 'test',
        isError: false,
        payload: {},
      }

      const keys = getInvalidationKeys(event)
      expect(keys).toHaveLength(1)
      expect(keys[0]).toBe(queryKeys.tasks.messages)
    })

    it('should return correct invalidation keys for WORKSPACE event', () => {
      const event: Event = {
        eventId: 'event-4',
        type: 'workspace',
        taskId: 'task-999',
        agentType: 'test',
        isError: false,
        payload: {},
      }

      const keys = getInvalidationKeys(event)

      expect(keys).toHaveLength(3)
      expect(keys[0]('task-999')).toEqual(queryKeys.tasks.workspaceStatus('task-999'))
      expect(keys[1]('task-999')).toEqual(queryKeys.tasks.workspaceSetup('task-999'))
      expect(keys[2]('task-999')).toEqual(queryKeys.tasks.gitDiff('task-999'))
    })
  })

  describe('shouldInvalidateTaskList', () => {
    it('should return true for status-affecting events', () => {
      const statusEvents = [{ type: 'error' }, { type: 'task_status' }]

      statusEvents.forEach((event) => {
        expect(shouldInvalidateTaskList(event as any)).toBe(true)
      })
    })

    it('should return false for non-status-affecting events', () => {
      const nonStatusEvents = [
        { type: 'message' },
        { type: 'workspace' },
        { type: 'tool' },
        { type: 'server' },
        { type: 'result' }, // RESULT no longer affects task status directly
      ]

      nonStatusEvents.forEach((event) => {
        expect(shouldInvalidateTaskList(event as any)).toBe(false)
      })
    })
  })

  describe('isStatusAffectingEvent', () => {
    it('should identify status-affecting events correctly', () => {
      STATUS_AFFECTING_EVENTS.forEach((eventType) => {
        expect(isStatusAffectingEvent({ type: eventType } as any)).toBe(true)
      })
    })
  })

  describe('EVENT_INVALIDATION_MAP', () => {
    it('should not include tasks.all invalidation for any event', () => {
      // Verify that no event type invalidates all tasks
      Object.entries(EVENT_INVALIDATION_MAP).forEach(([_eventType, invalidationFns]) => {
        invalidationFns.forEach((fn) => {
          const result = fn('test-task-id')
          // Ensure no invalidation key equals ['tasks'] which would invalidate all tasks
          expect(result).not.toEqual(['tasks'])
          expect(result).not.toEqual(queryKeys.tasks.all)
        })
      })
    })

    it('should only invalidate specific task resources', () => {
      // Verify TASK_STATUS only invalidates the specific task
      const taskStatusInvalidations = EVENT_INVALIDATION_MAP['task_status']
      expect(taskStatusInvalidations).toHaveLength(1)
      expect(taskStatusInvalidations[0]('task-123')).toEqual(queryKeys.tasks.detail('task-123'))
    })
  })
})
