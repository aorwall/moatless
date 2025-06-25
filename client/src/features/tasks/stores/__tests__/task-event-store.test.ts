import type { Event } from '@shared/types/event'
import { beforeEach, describe, expect, it } from 'vitest'
import { useTaskEventStore } from '../task-event-store'

describe('useTaskEventStore', () => {
  beforeEach(() => {
    // Clear the store before each test
    const store = useTaskEventStore.getState()
    store.eventTasks.clear()
  })

  describe('SDK Message Events', () => {
    it('should store SDK assistant message events', () => {
      const taskId = '7f5ddcd4-ca18-485a-9306-4b91ef6782d8'
      const event: Event = {
        eventId: 'e81f1d4c-d470-4bf0-bf53-8e20468fb922',
        type: 'message',
        taskId,
        iterationId: 1,
        timestamp: '2025-06-25T12:38:37.087Z',
        isError: false,
        payload: {
          type: 'assistant',
          message: {
            id: 'msg_01CXrZbZ8ia8wwcoxJKnAQ64',
            type: 'message',
            role: 'assistant',
            model: 'claude-opus-4-20250514',
            content: [
              {
                type: 'text',
                text: 'I see you have a React TypeScript project with Vite. Let me modify the App.tsx file to create a simple hello world app.',
              },
            ],
            stop_reason: null,
            stop_sequence: null,
            usage: {
              input_tokens: 6,
              cache_creation_input_tokens: 688,
              cache_read_input_tokens: 13502,
              output_tokens: 1,
              service_tier: 'standard',
            },
          },
          parent_tool_use_id: null,
          session_id: 'ed9da787-6508-413a-87bb-7c7ed8915897',
        },
      }

      const store = useTaskEventStore.getState()
      store.addStreamEvent(taskId, event)

      const taskState = store.getTaskEventState(taskId)
      expect(taskState.streamEvents).toHaveLength(1)
      expect(taskState.streamEvents[0]).toEqual(event)
      expect(taskState.lastEventId).toBe(event.eventId)
    })
  })
})
