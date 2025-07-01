import type { ClaudeCodeSDKMessage } from '@moatless/shared/types/claude'
import type { Event, MessageEvent } from '@moatless/shared/types/event'
import { describe, expect, it, vi } from 'vitest'

import { mergeToolEvents } from '../merge-tool-events'

describe('mergeToolEvents', () => {
  it('should merge tool call with matching tool result', () => {
    const events: Event[] = [
      {
        eventId: 'event-1',
        type: 'message',
        subType: 'tool_call',
        taskId: 'task-1',
        iterationId: 1,
        timestamp: '2025-01-01T00:00:00Z',
        isError: false,
        contextTokens: 100,
        numTurns: 1,
        totalCost: 0.01,
        payload: {
          type: 'assistant',
          message: {
            role: 'assistant',
            content: [
              {
                type: 'tool_use',
                id: 'tool-use-123',
                name: 'Read',
                input: { file_path: '/test.ts' },
              },
            ],
          },
        } as ClaudeCodeSDKMessage,
      } as MessageEvent,
      {
        eventId: 'event-2',
        type: 'message',
        subType: '',
        taskId: 'task-1',
        iterationId: 1,
        timestamp: '2025-01-01T00:00:01Z',
        isError: false,
        contextTokens: 100,
        numTurns: 1,
        totalCost: 0.01,
        payload: {
          type: 'user',
          message: {
            role: 'user',
            content: [
              {
                type: 'tool_result',
                tool_use_id: 'tool-use-123',
                content: 'File content here',
              },
            ],
          },
        } as ClaudeCodeSDKMessage,
      } as MessageEvent,
    ]

    const merged = mergeToolEvents(events)

    expect(merged).toHaveLength(1)
    expect(merged[0]).toMatchObject({
      eventId: 'event-1-tool-use-123',
      toolResult: {
        eventId: 'event-2',
      },
    })
  })

  it('should keep tool calls without results', () => {
    const events: Event[] = [
      {
        eventId: 'event-1',
        type: 'message',
        subType: 'tool_call',
        taskId: 'task-1',
        iterationId: 1,
        timestamp: '2025-01-01T00:00:00Z',
        isError: false,
        contextTokens: 100,
        numTurns: 1,
        totalCost: 0.01,
        payload: {
          type: 'assistant',
          message: {
            role: 'assistant',
            content: [
              {
                type: 'tool_use',
                id: 'tool-use-456',
                name: 'Read',
                input: { file_path: '/test.ts' },
              },
            ],
          },
        } as ClaudeCodeSDKMessage,
      } as MessageEvent,
    ]

    const merged = mergeToolEvents(events)

    expect(merged).toHaveLength(1)
    expect(merged[0]).toMatchObject({
      eventId: 'event-1',
    })
    expect('toolResult' in merged[0]).toBe(false)
  })

  it('should handle non-tool events correctly', () => {
    const events: Event[] = [
      {
        eventId: 'event-1',
        type: 'message',
        subType: '',
        taskId: 'task-1',
        iterationId: 1,
        timestamp: '2025-01-01T00:00:00Z',
        isError: false,
        contextTokens: 100,
        numTurns: 1,
        totalCost: 0.01,
        payload: {
          type: 'assistant',
          message: {
            role: 'assistant',
            content: [
              {
                type: 'text',
                text: 'Hello world',
              },
            ],
          },
        } as ClaudeCodeSDKMessage,
      } as MessageEvent,
      {
        eventId: 'event-2',
        type: 'input',
        subType: '',
        taskId: 'task-1',
        iterationId: 1,
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
    expect(merged[0].eventId).toBe('event-1')
    expect(merged[1].eventId).toBe('event-2')
  })

  it('should handle multiple tool calls and results', () => {
    const events: Event[] = [
      {
        eventId: 'event-1',
        type: 'message',
        subType: 'tool_call',
        taskId: 'task-1',
        iterationId: 1,
        timestamp: '2025-01-01T00:00:00Z',
        isError: false,
        contextTokens: 100,
        numTurns: 1,
        totalCost: 0.01,
        payload: {
          type: 'assistant',
          message: {
            role: 'assistant',
            content: [
              {
                type: 'tool_use',
                id: 'tool-1',
                name: 'Read',
                input: { file_path: '/test1.ts' },
              },
            ],
          },
        } as ClaudeCodeSDKMessage,
      } as MessageEvent,
      {
        eventId: 'event-2',
        type: 'message',
        subType: '',
        taskId: 'task-1',
        iterationId: 1,
        timestamp: '2025-01-01T00:00:01Z',
        isError: false,
        contextTokens: 100,
        numTurns: 1,
        totalCost: 0.01,
        payload: {
          type: 'user',
          message: {
            role: 'user',
            content: [
              {
                type: 'tool_result',
                tool_use_id: 'tool-1',
                content: 'Content 1',
              },
            ],
          },
        } as ClaudeCodeSDKMessage,
      } as MessageEvent,
      {
        eventId: 'event-3',
        type: 'message',
        subType: 'tool_call',
        taskId: 'task-1',
        iterationId: 1,
        timestamp: '2025-01-01T00:00:02Z',
        isError: false,
        contextTokens: 100,
        numTurns: 1,
        totalCost: 0.01,
        payload: {
          type: 'assistant',
          message: {
            role: 'assistant',
            content: [
              {
                type: 'tool_use',
                id: 'tool-2',
                name: 'Write',
                input: { file_path: '/test2.ts', content: 'new content' },
              },
            ],
          },
        } as ClaudeCodeSDKMessage,
      } as MessageEvent,
      {
        eventId: 'event-4',
        type: 'message',
        subType: '',
        taskId: 'task-1',
        iterationId: 1,
        timestamp: '2025-01-01T00:00:03Z',
        isError: false,
        contextTokens: 100,
        numTurns: 1,
        totalCost: 0.01,
        payload: {
          type: 'user',
          message: {
            role: 'user',
            content: [
              {
                type: 'tool_result',
                tool_use_id: 'tool-2',
                content: 'Write successful',
              },
            ],
          },
        } as ClaudeCodeSDKMessage,
      } as MessageEvent,
    ]

    const merged = mergeToolEvents(events)

    expect(merged).toHaveLength(2)
    expect(merged[0]).toMatchObject({
      eventId: 'event-1-tool-1',
      toolResult: {
        eventId: 'event-2',
      },
    })
    expect(merged[1]).toMatchObject({
      eventId: 'event-3-tool-2',
      toolResult: {
        eventId: 'event-4',
      },
    })
  })

  it('should detect tool calls without subType by checking content', () => {
    const events: Event[] = [
      {
        eventId: 'event-1',
        type: 'message',
        subType: '', // No subType, should detect by content
        taskId: 'task-1',
        iterationId: 1,
        timestamp: '2025-01-01T00:00:00Z',
        isError: false,
        contextTokens: 100,
        numTurns: 1,
        totalCost: 0.01,
        payload: {
          type: 'assistant',
          message: {
            role: 'assistant',
            content: [
              {
                type: 'tool_use',
                id: 'tool-use-789',
                name: 'Bash',
                input: { command: 'ls -la' },
              },
            ],
          },
        } as ClaudeCodeSDKMessage,
      },
      {
        eventId: 'event-2',
        type: 'message',
        subType: '', // empty subType should also work
        taskId: 'task-1',
        iterationId: 1,
        timestamp: '2025-01-01T00:00:01Z',
        isError: false,
        contextTokens: 100,
        numTurns: 1,
        totalCost: 0.01,
        payload: {
          type: 'user',
          message: {
            role: 'user',
            content: [
              {
                type: 'tool_result',
                tool_use_id: 'tool-use-789',
                content: 'total 64\ndrwxr-xr-x  ...',
              },
            ],
          },
        } as ClaudeCodeSDKMessage,
      },
    ]

    const merged = mergeToolEvents(events)

    expect(merged).toHaveLength(1)
    expect(merged[0]).toMatchObject({
      eventId: 'event-1-tool-use-789',
      toolResult: {
        eventId: 'event-2',
      },
    })
  })

  it('should handle mixed tool and non-tool content in the same message', () => {
    const events: Event[] = [
      {
        eventId: 'event-1',
        type: 'message',
        subType: '',
        taskId: 'task-1',
        iterationId: 1,
        timestamp: '2025-01-01T00:00:00Z',
        isError: false,
        contextTokens: 100,
        numTurns: 1,
        totalCost: 0.01,
        payload: {
          type: 'assistant',
          message: {
            role: 'assistant',
            content: [
              {
                type: 'text',
                text: 'Let me check the file for you.',
              },
              {
                type: 'tool_use',
                id: 'tool-mixed-1',
                name: 'Read',
                input: { file_path: '/config.ts' },
              },
            ],
          },
        } as ClaudeCodeSDKMessage,
      } as MessageEvent,
      {
        eventId: 'event-2',
        type: 'message',
        subType: '',
        taskId: 'task-1',
        iterationId: 1,
        timestamp: '2025-01-01T00:00:01Z',
        isError: false,
        contextTokens: 100,
        numTurns: 1,
        totalCost: 0.01,
        payload: {
          type: 'user',
          message: {
            role: 'user',
            content: [
              {
                type: 'tool_result',
                tool_use_id: 'tool-mixed-1',
                content: 'Config file contents',
              },
            ],
          },
        } as ClaudeCodeSDKMessage,
      } as MessageEvent,
    ]

    const merged = mergeToolEvents(events)

    expect(merged).toHaveLength(1)
    expect(merged[0]).toMatchObject({
      eventId: 'event-1-tool-mixed-1',
      toolResult: {
        eventId: 'event-2',
      },
    })
    // Ensure the text content is still present
    const mergedPayload = merged[0].payload as Record<string, unknown>
    expect(mergedPayload.message.content[0]).toMatchObject({
      type: 'text',
      text: 'Let me check the file for you.',
    })
  })

  it('should handle orphaned tool results (no matching tool call)', () => {
    const events: Event[] = [
      {
        eventId: 'event-1',
        type: 'message',
        subType: '',
        taskId: 'task-1',
        iterationId: 1,
        timestamp: '2025-01-01T00:00:00Z',
        isError: false,
        contextTokens: 100,
        numTurns: 1,
        totalCost: 0.01,
        payload: {
          type: 'user',
          message: {
            role: 'user',
            content: [
              {
                type: 'tool_result',
                tool_use_id: 'non-existent-tool',
                content: 'Orphaned result',
              },
            ],
          },
        } as ClaudeCodeSDKMessage,
      } as MessageEvent,
    ]

    const merged = mergeToolEvents(events)

    // Orphaned tool results should be kept as-is
    expect(merged).toHaveLength(1)
    expect(merged[0].eventId).toBe('event-1')
    expect('toolResult' in merged[0]).toBe(false)
  })

  describe('logging behavior', () => {
    beforeEach(() => {
      vi.spyOn(console, 'warn').mockImplementation(() => {})
    })

    afterEach(() => {
      vi.restoreAllMocks()
    })

    it('should log warning for unmatched tool calls', () => {
      const events: Event[] = [
        {
          eventId: 'event-1',
          type: 'message',
          subType: '',
          taskId: 'task-1',
          iterationId: 1,
          timestamp: '2025-01-01T00:00:00Z',
          isError: false,
          payload: {
            type: 'assistant',
            message: {
              role: 'assistant',
              content: [
                {
                  type: 'tool_use',
                  id: 'unmatched-tool-1',
                  name: 'Read',
                  input: { file_path: '/test.ts' },
                },
              ],
            },
          } as ClaudeCodeSDKMessage,
        } as MessageEvent,
      ]

      mergeToolEvents(events)

      expect(console.warn).toHaveBeenCalledWith(
        '[mergeToolEvents] Found 1 tool calls without matching results:',
        ['unmatched-tool-1'],
      )
    })

    it('should log warning for orphaned tool results', () => {
      const events: Event[] = [
        {
          eventId: 'event-1',
          type: 'message',
          subType: '',
          taskId: 'task-1',
          iterationId: 1,
          timestamp: '2025-01-01T00:00:00Z',
          isError: false,
          payload: {
            type: 'user',
            message: {
              role: 'user',
              content: [
                {
                  type: 'tool_result',
                  tool_use_id: 'orphan-tool-1',
                  content: 'Result without call',
                },
              ],
            },
          } as ClaudeCodeSDKMessage,
        } as MessageEvent,
      ]

      mergeToolEvents(events)

      expect(console.warn).toHaveBeenCalledWith(
        '[mergeToolEvents] Found 1 orphaned tool results:',
        ['orphan-tool-1'],
      )
    })

    it('should log warning for invalid payload structure', () => {
      const events: Event[] = [
        {
          eventId: 'event-bad',
          type: 'message',
          subType: '',
          taskId: 'task-1',
          iterationId: 1,
          timestamp: '2025-01-01T00:00:00Z',
          isError: false,
          payload: null as unknown as ClaudeCodeSDKMessage,
        } as MessageEvent,
      ]

      const merged = mergeToolEvents(events)

      expect(console.warn).toHaveBeenCalledWith(
        '[mergeToolEvents] MessageEvent event-bad has invalid payload structure',
        expect.any(Object),
      )
      expect(merged).toHaveLength(1)
    })

    it('should log warning for unexpected message type', () => {
      const events: Event[] = [
        {
          eventId: 'event-1',
          type: 'message',
          subType: '',
          taskId: 'task-1',
          iterationId: 1,
          timestamp: '2025-01-01T00:00:00Z',
          isError: false,
          payload: {
            type: 'system',
            message: {
              role: 'system' as unknown as 'assistant' | 'user',
              content: 'System message',
            },
          } as unknown as ClaudeCodeSDKMessage,
        } as MessageEvent,
      ]

      const merged = mergeToolEvents(events)

      // Should handle gracefully without throwing and include the event
      expect(merged).toHaveLength(1)
      expect(merged[0]).toMatchObject({
        eventId: 'event-1',
      })
    })
  })
})
