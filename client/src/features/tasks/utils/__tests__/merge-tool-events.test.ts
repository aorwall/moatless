import { describe, expect, it } from 'vitest'

import type { Event } from '@/types/event'

import { mergeToolEvents } from '../merge-tool-events'

describe('mergeToolEvents', () => {
  it('should merge tool call with matching tool result', () => {
    const events: Event[] = [
      {
        event_id: 'event-1',
        type: 'message',
        subType: 'tool_call',
        taskId: 'task-1',
        iterationNumber: 1,
        timestamp: '2025-01-01T00:00:00Z',
        isError: false,
        contextTokens: 100,
        numTurns: 1,
        totalCost: 0.01,
        payload: {
          type: 'assistant',
          message: {
            content: [
              {
                type: 'tool_use',
                id: 'tool-use-123',
                name: 'Read',
                input: { file_path: '/test.ts' },
              },
            ],
          },
        },
      },
      {
        event_id: 'event-2',
        type: 'message',
        subType: '',
        taskId: 'task-1',
        iterationNumber: 1,
        timestamp: '2025-01-01T00:00:01Z',
        isError: false,
        contextTokens: 100,
        numTurns: 1,
        totalCost: 0.01,
        payload: {
          type: 'user',
          message: {
            content: [
              {
                type: 'tool_result',
                tool_use_id: 'tool-use-123',
                content: 'File content here',
              },
            ],
          },
        },
      },
    ]

    const merged = mergeToolEvents(events)

    expect(merged).toHaveLength(1)
    expect(merged[0]).toMatchObject({
      event_id: 'event-1',
      toolResult: {
        event_id: 'event-2',
      },
    })
  })

  it('should keep tool calls without results', () => {
    const events: Event[] = [
      {
        event_id: 'event-1',
        type: 'message',
        subType: 'tool_call',
        taskId: 'task-1',
        iterationNumber: 1,
        timestamp: '2025-01-01T00:00:00Z',
        isError: false,
        contextTokens: 100,
        numTurns: 1,
        totalCost: 0.01,
        payload: {
          type: 'assistant',
          message: {
            content: [
              {
                type: 'tool_use',
                id: 'tool-use-456',
                name: 'Read',
                input: { file_path: '/test.ts' },
              },
            ],
          },
        },
      },
    ]

    const merged = mergeToolEvents(events)

    expect(merged).toHaveLength(1)
    expect(merged[0]).toMatchObject({
      event_id: 'event-1',
    })
    expect('toolResult' in merged[0]).toBe(false)
  })

  it('should handle non-tool events correctly', () => {
    const events: Event[] = [
      {
        event_id: 'event-1',
        type: 'message',
        subType: '',
        taskId: 'task-1',
        iterationNumber: 1,
        timestamp: '2025-01-01T00:00:00Z',
        isError: false,
        contextTokens: 100,
        numTurns: 1,
        totalCost: 0.01,
        payload: {
          type: 'assistant',
          message: {
            content: [
              {
                type: 'text',
                text: 'Hello world',
              },
            ],
          },
        },
      },
      {
        event_id: 'event-2',
        type: 'input',
        subType: '',
        taskId: 'task-1',
        iterationNumber: 1,
        timestamp: '2025-01-01T00:00:01Z',
        isError: false,
        contextTokens: 100,
        numTurns: 1,
        totalCost: 0.01,
        payload: {
          message: 'User input',
        },
      },
    ]

    const merged = mergeToolEvents(events)

    expect(merged).toHaveLength(2)
    expect(merged[0].event_id).toBe('event-1')
    expect(merged[1].event_id).toBe('event-2')
  })

  it('should handle multiple tool calls and results', () => {
    const events: Event[] = [
      {
        event_id: 'event-1',
        type: 'message',
        subType: 'tool_call',
        taskId: 'task-1',
        iterationNumber: 1,
        timestamp: '2025-01-01T00:00:00Z',
        isError: false,
        contextTokens: 100,
        numTurns: 1,
        totalCost: 0.01,
        payload: {
          type: 'assistant',
          message: {
            content: [
              {
                type: 'tool_use',
                id: 'tool-1',
                name: 'Read',
                input: { file_path: '/test1.ts' },
              },
            ],
          },
        },
      },
      {
        event_id: 'event-2',
        type: 'message',
        subType: '',
        taskId: 'task-1',
        iterationNumber: 1,
        timestamp: '2025-01-01T00:00:01Z',
        isError: false,
        contextTokens: 100,
        numTurns: 1,
        totalCost: 0.01,
        payload: {
          type: 'user',
          message: {
            content: [
              {
                type: 'tool_result',
                tool_use_id: 'tool-1',
                content: 'Content 1',
              },
            ],
          },
        },
      },
      {
        event_id: 'event-3',
        type: 'message',
        subType: 'tool_call',
        taskId: 'task-1',
        iterationNumber: 1,
        timestamp: '2025-01-01T00:00:02Z',
        isError: false,
        contextTokens: 100,
        numTurns: 1,
        totalCost: 0.01,
        payload: {
          type: 'assistant',
          message: {
            content: [
              {
                type: 'tool_use',
                id: 'tool-2',
                name: 'Write',
                input: { file_path: '/test2.ts', content: 'new content' },
              },
            ],
          },
        },
      },
      {
        event_id: 'event-4',
        type: 'message',
        subType: '',
        taskId: 'task-1',
        iterationNumber: 1,
        timestamp: '2025-01-01T00:00:03Z',
        isError: false,
        contextTokens: 100,
        numTurns: 1,
        totalCost: 0.01,
        payload: {
          type: 'user',
          message: {
            content: [
              {
                type: 'tool_result',
                tool_use_id: 'tool-2',
                content: 'Write successful',
              },
            ],
          },
        },
      },
    ]

    const merged = mergeToolEvents(events)

    expect(merged).toHaveLength(2)
    expect(merged[0]).toMatchObject({
      event_id: 'event-1',
      toolResult: {
        event_id: 'event-2',
      },
    })
    expect(merged[1]).toMatchObject({
      event_id: 'event-3',
      toolResult: {
        event_id: 'event-4',
      },
    })
  })
})
