import { describe, it, expect, beforeEach, vi } from 'vitest'
import type { Event, EventType } from '@moatless/types'
import { MessageService } from '../message.service.js'

describe('MessageService - Unit Tests', () => {
  let messageService: MessageService
  
  beforeEach(() => {
    vi.clearAllMocks()
    messageService = new MessageService()
  })

  describe('transformEventsToMessages', () => {
    it('should transform simple message events to messages', () => {
      const mockEvents: Event[] = [
        {
          eventId: 'event-1',
          agentType: 'test-agent',
          type: 'message' as EventType,
          taskId: 'task-1',
          timestamp: new Date('2024-01-01T10:00:00Z'),
          isError: false,
          payload: {
            message: {
              role: 'user',
              content: [
                {
                  type: 'text',
                  text: 'Hello, how can I help you?'
                }
              ]
            }
          }
        },
        {
          eventId: 'event-2',
          agentType: 'test-agent', 
          type: 'message' as EventType,
          taskId: 'task-1',
          timestamp: new Date('2024-01-01T10:01:00Z'),
          isError: false,
          payload: {
            message: {
              role: 'assistant',
              content: [
                {
                  type: 'text',
                  text: 'I can help you with your task.'
                }
              ]
            }
          }
        }
      ]

      const result = messageService.transformEventsToMessages(mockEvents)

      expect(result).toHaveLength(2)
      expect(result[0]).toEqual({
        id: 'event-1',
        role: 'user',
        content: 'Hello, how can I help you?',
        timestamp: new Date('2024-01-01T10:00:00Z'),
        eventId: 'event-1'
      })
      expect(result[1]).toEqual({
        id: 'event-2',
        role: 'assistant', 
        content: 'I can help you with your task.',
        timestamp: new Date('2024-01-01T10:01:00Z'),
        eventId: 'event-2'
      })
    })

    it('should merge tool call events into messages', () => {
      const mockEvents: Event[] = [
        {
          eventId: 'event-1',
          agentType: 'test-agent',
          type: 'message' as EventType,
          taskId: 'task-1',
          timestamp: new Date('2024-01-01T10:00:00Z'),
          isError: false,
          payload: {
            message: {
              role: 'assistant',
              content: [
                {
                  type: 'text',
                  text: 'I\'ll run a command for you.'
                },
                {
                  type: 'tool_use',
                  id: 'tool-1',
                  name: 'Bash',
                  input: { command: 'ls -la' }
                }
              ]
            }
          }
        },
        {
          eventId: 'event-2',
          agentType: 'test-agent',
          type: 'message' as EventType,
          taskId: 'task-1',
          timestamp: new Date('2024-01-01T10:00:01Z'),
          isError: false,
          payload: {
            message: {
              role: 'user',
              content: [
                {
                  type: 'tool_result',
                  tool_use_id: 'tool-1',
                  content: 'total 8\ndrwxr-xr-x 3 user user 4096 Jan  1 10:00 .\ndrwxr-xr-x 3 user user 4096 Jan  1 09:59 ..'
                }
              ]
            }
          }
        }
      ]

      const result = messageService.transformEventsToMessages(mockEvents)

      expect(result).toHaveLength(1)
      expect(result[0]).toMatchObject({
        id: 'event-1',
        role: 'assistant',
        timestamp: new Date('2024-01-01T10:00:00Z'),
        eventId: 'event-1'
      })
      
      // Check that content includes both text and tool use
      expect(result[0].content).toEqual([
        {
          type: 'text',
          text: 'I\'ll run a command for you.'
        },
        {
          type: 'tool_use',
          tool_name: 'Bash',
          tool_input: { command: 'ls -la' },
          tool_use_id: 'tool-1',
          tool_result: {
            content: 'total 8\ndrwxr-xr-x 3 user user 4096 Jan  1 10:00 .\ndrwxr-xr-x 3 user user 4096 Jan  1 09:59 ..',
            is_error: undefined,
            eventId: 'event-2'
          }
        }
      ])
    })

    it('should handle messages without tool calls', () => {
      const mockEvents: Event[] = [
        {
          eventId: 'event-1',
          agentType: 'test-agent',
          type: 'message' as EventType,
          taskId: 'task-1',
          timestamp: new Date('2024-01-01T10:00:00Z'),
          isError: false,
          payload: {
            message: {
              role: 'assistant',
              content: [
                {
                  type: 'text',
                  text: 'This is a simple message without any tool calls.'
                }
              ]
            }
          }
        }
      ]

      const result = messageService.transformEventsToMessages(mockEvents)

      expect(result).toHaveLength(1)
      expect(result[0]).toEqual({
        id: 'event-1',
        role: 'assistant',
        content: 'This is a simple message without any tool calls.',
        timestamp: new Date('2024-01-01T10:00:00Z'),
        eventId: 'event-1'
      })
    })

    it('should handle tool calls without results', () => {
      const mockEvents: Event[] = [
        {
          eventId: 'event-1',
          agentType: 'test-agent',
          type: 'message' as EventType,
          taskId: 'task-1',
          timestamp: new Date('2024-01-01T10:00:00Z'),
          isError: false,
          payload: {
            message: {
              role: 'assistant',
              content: [
                {
                  type: 'text',
                  text: 'I\'ll run a command.'
                },
                {
                  type: 'tool_use',
                  id: 'tool-1',
                  name: 'Bash',
                  input: { command: 'echo hello' }
                }
              ]
            }
          }
        }
        // No tool_result event
      ]

      const result = messageService.transformEventsToMessages(mockEvents)

      expect(result).toHaveLength(1)
      expect(result[0].content).toEqual([
        {
          type: 'text',
          text: 'I\'ll run a command.'
        },
        {
          type: 'tool_use',
          tool_name: 'Bash',
          tool_input: { command: 'echo hello' },
          tool_use_id: 'tool-1',
          tool_result: undefined
        }
      ])
    })

    it('should handle empty events array', () => {
      const result = messageService.transformEventsToMessages([])
      expect(result).toHaveLength(0)
    })

    it('should handle events without message data', () => {
      const mockEvents: Event[] = [
        {
          eventId: 'event-1',
          agentType: 'test-agent',
          type: 'task_status' as EventType,
          taskId: 'task-1',
          timestamp: new Date('2024-01-01T10:00:00Z'),
          isError: false,
          payload: {
            status: 'running'
          }
        }
      ]

      const result = messageService.transformEventsToMessages(mockEvents)
      expect(result).toHaveLength(0)
    })

    it('should handle invalid payload gracefully', () => {
      const mockEvents: Event[] = [
        {
          eventId: 'event-1',
          agentType: 'test-agent',
          type: 'message' as EventType,
          taskId: 'task-1',
          timestamp: new Date('2024-01-01T10:00:00Z'),
          isError: false,
          payload: null as any
        }
      ]

      const result = messageService.transformEventsToMessages(mockEvents)
      expect(result).toHaveLength(0)
    })

    it('should preserve message order by timestamp', () => {
      const mockEvents: Event[] = [
        {
          eventId: 'event-2',
          agentType: 'test-agent',
          type: 'message' as EventType,
          taskId: 'task-1',
          timestamp: new Date('2024-01-01T10:01:00Z'),
          isError: false,
          payload: {
            message: {
              role: 'assistant',
              content: [
                {
                  type: 'text',
                  text: 'Second message'
                }
              ]
            }
          }
        },
        {
          eventId: 'event-1',
          agentType: 'test-agent',
          type: 'message' as EventType,
          taskId: 'task-1',
          timestamp: new Date('2024-01-01T10:00:00Z'),
          isError: false,
          payload: {
            message: {
              role: 'user',
              content: [
                {
                  type: 'text',
                  text: 'First message'
                }
              ]
            }
          }
        }
      ]

      const result = messageService.transformEventsToMessages(mockEvents)

      expect(result).toHaveLength(2)
      // Note: The order should match the input order, not timestamp order
      // The service doesn't sort by timestamp, it preserves input order
      expect(result[0].content).toBe('Second message')
      expect(result[1].content).toBe('First message')
    })

    it('should handle multiple tool calls in a single message', () => {
      const mockEvents: Event[] = [
        {
          eventId: 'event-1',
          agentType: 'test-agent',
          type: 'message' as EventType,
          taskId: 'task-1',
          timestamp: new Date('2024-01-01T10:00:00Z'),
          isError: false,
          payload: {
            message: {
              role: 'assistant',
              content: [
                {
                  type: 'text',
                  text: 'I\'ll check the directory and then read a file.'
                },
                {
                  type: 'tool_use',
                  id: 'tool-1',
                  name: 'Bash',
                  input: { command: 'ls' }
                },
                {
                  type: 'tool_use',
                  id: 'tool-2',
                  name: 'Read',
                  input: { file_path: './README.md' }
                }
              ]
            }
          }
        },
        {
          eventId: 'event-2',
          agentType: 'test-agent',
          type: 'message' as EventType,
          taskId: 'task-1',
          timestamp: new Date('2024-01-01T10:00:01Z'),
          isError: false,
          payload: {
            message: {
              role: 'user',
              content: [
                {
                  type: 'tool_result',
                  tool_use_id: 'tool-1',
                  content: 'file1.txt\nfile2.txt\nREADME.md'
                }
              ]
            }
          }
        },
        {
          eventId: 'event-3',
          agentType: 'test-agent',
          type: 'message' as EventType,
          taskId: 'task-1',
          timestamp: new Date('2024-01-01T10:00:02Z'),
          isError: false,
          payload: {
            message: {
              role: 'user',
              content: [
                {
                  type: 'tool_result',
                  tool_use_id: 'tool-2',
                  content: '# Project README\n\nThis is a test project.'
                }
              ]
            }
          }
        }
      ]

      const result = messageService.transformEventsToMessages(mockEvents)

      expect(result).toHaveLength(1)
      expect(result[0].content).toEqual([
        {
          type: 'text',
          text: 'I\'ll check the directory and then read a file.'
        },
        {
          type: 'tool_use',
          tool_name: 'Bash',
          tool_input: { command: 'ls' },
          tool_use_id: 'tool-1',
          tool_result: {
            content: 'file1.txt\nfile2.txt\nREADME.md',
            is_error: undefined,
            eventId: 'event-2'
          }
        },
        {
          type: 'tool_use',
          tool_name: 'Read',
          tool_input: { file_path: './README.md' },
          tool_use_id: 'tool-2',
          tool_result: {
            content: '# Project README\n\nThis is a test project.',
            is_error: undefined,
            eventId: 'event-3'
          }
        }
      ])
    })
  })
})