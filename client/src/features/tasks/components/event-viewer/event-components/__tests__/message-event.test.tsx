import type { Event } from '@shared/types/event'
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { MessageEvent } from '../message-event'

describe('MessageEvent', () => {
  const createMessageEvent = (payload: Record<string, unknown>): Event => ({
    event_id: 'test-event',
    type: 'message',
    subType: '',
    taskId: 'test-task',
    iterationNumber: 1,
    timestamp: '2025-01-01T00:00:00Z',
    isError: false,
    contextTokens: 100,
    numTurns: 1,
    totalCost: 0.01,
    payload,
  })

  it('should render assistant message with bot icon', () => {
    const event = createMessageEvent({
      type: 'assistant',
      message: {
        content: [
          {
            type: 'text',
            text: 'Hello from assistant',
          },
        ],
      },
    })

    render(<MessageEvent event={event} />)

    expect(screen.getByText('Assistant')).toBeInTheDocument()
    expect(screen.getByText('Hello from assistant')).toBeInTheDocument()
    // Bot icon is rendered
    const botIcon = document.querySelector('svg.lucide-bot')
    expect(botIcon).toBeInTheDocument()
  })

  it('should render user message with user icon', () => {
    const event = createMessageEvent({
      type: 'user',
      message: {
        content: [
          {
            type: 'text',
            text: 'Hello from user',
          },
        ],
      },
    })

    render(<MessageEvent event={event} />)

    expect(screen.getByText('User')).toBeInTheDocument()
    expect(screen.getByText('Hello from user')).toBeInTheDocument()
    // User icon is rendered
    const userIcon = document.querySelector('svg.lucide-user')
    expect(userIcon).toBeInTheDocument()
  })

  it('should render markdown content', () => {
    const event = createMessageEvent({
      type: 'assistant',
      message: {
        content: [
          {
            type: 'text',
            text: '# Heading\n\nThis is **bold** text with `inline code`',
          },
        ],
      },
    })

    render(<MessageEvent event={event} />)

    // Markdown is rendered
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('Heading')
    expect(screen.getByText('bold')).toBeInTheDocument()
    expect(screen.getByText('inline code')).toBeInTheDocument()
  })

  it('should handle string content directly', () => {
    const event = createMessageEvent({
      type: 'user',
      message: {
        content: 'Direct string content',
      },
    })

    render(<MessageEvent event={event} />)

    expect(screen.getByText('Direct string content')).toBeInTheDocument()
  })

  it('should handle message.text field', () => {
    const event = createMessageEvent({
      type: 'assistant',
      message: {
        text: 'Text field content',
      },
    })

    render(<MessageEvent event={event} />)

    expect(screen.getByText('Text field content')).toBeInTheDocument()
  })

  it('should handle message.message field', () => {
    const event = createMessageEvent({
      type: 'user',
      message: {
        message: 'Message field content',
      },
    })

    render(<MessageEvent event={event} />)

    expect(screen.getByText('Message field content')).toBeInTheDocument()
  })

  it('should show no content message when text is empty', () => {
    const event = createMessageEvent({
      type: 'assistant',
      message: {
        content: [],
      },
    })

    render(<MessageEvent event={event} />)

    expect(screen.getByText('No text content')).toBeInTheDocument()
  })

  it('should show no message content when message is null', () => {
    const event = createMessageEvent({
      type: 'assistant',
    })

    render(<MessageEvent event={event} />)

    expect(screen.getByText('No message content')).toBeInTheDocument()
  })

  it('should handle code blocks in markdown', () => {
    const event = createMessageEvent({
      type: 'assistant',
      message: {
        content: [
          {
            type: 'text',
            text: 'Here is some code:\n\n```javascript\nconst hello = "world";\n```',
          },
        ],
      },
    })

    render(<MessageEvent event={event} />)

    expect(screen.getByText('const hello = "world";')).toBeInTheDocument()
    // Code block should be in a pre element
    const preElement = screen.getByText('const hello = "world";').closest('pre')
    expect(preElement).toHaveClass('bg-muted', 'rounded-md', 'p-3')
  })

  describe('SDK Message format', () => {
    it('should handle SDK assistant message with content array', () => {
      const event: Event = {
        eventId: 'e81f1d4c-d470-4bf0-bf53-8e20468fb922',
        type: 'message',
        taskId: '7f5ddcd4-ca18-485a-9306-4b91ef6782d8',
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

      render(<MessageEvent event={event} />)

      expect(screen.getByText('Assistant')).toBeInTheDocument()
      expect(
        screen.getByText(
          'I see you have a React TypeScript project with Vite. Let me modify the App.tsx file to create a simple hello world app.',
        ),
      ).toBeInTheDocument()
    })

    it('should handle SDK tool use message', () => {
      const event: Event = {
        eventId: 'b9e99242-2bef-4dc9-b47a-56028303a971',
        type: 'message',
        taskId: '7f5ddcd4-ca18-485a-9306-4b91ef6782d8',
        iterationId: 1,
        timestamp: '2025-06-25T12:38:37.552Z',
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
                type: 'tool_use',
                id: 'toolu_01FQfZbk5ZYnMxn1y3YGZffU',
                name: 'Read',
                input: {
                  file_path:
                    '/Users/albert/repos/moatless/templates/bolt-workspace-7f5ddcd4-ca18-485a-9306-4b91ef6782d8/src/App.tsx',
                },
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

      render(<MessageEvent event={event} />)

      // Tool use messages should not show text content
      expect(screen.getByText('No text content')).toBeInTheDocument()
    })

    it('should handle SDK user message with tool result', () => {
      const event: Event = {
        eventId: 'a2370d8d-c3de-4e31-8ebb-77a91eb27cfc',
        type: 'message',
        taskId: '7f5ddcd4-ca18-485a-9306-4b91ef6782d8',
        iterationId: 1,
        timestamp: '2025-06-25T12:38:38.105Z',
        isError: false,
        payload: {
          type: 'user',
          message: {
            role: 'user',
            content: [
              {
                tool_use_id: 'toolu_01FQfZbk5ZYnMxn1y3YGZffU',
                type: 'tool_result',
                content:
                  '     1→import React from \'react\';\n     2→\n     3→function App() {\n     4→  return (\n     5→    <div className="min-h-screen bg-gray-100 flex items-center justify-center">\n     6→      <div className="text-center">\n     7→        <h1 className="text-6xl font-bold text-gray-300">Your App</h1>\n     8→        <p className="text-gray-400 mt-4">Start building something amazing</p>\n     9→      </div>\n    10→    </div>\n    11→  );\n    12→}\n    13→\n    14→export default App;\n\n<system-reminder>\nWhenever you read a file, you should consider whether it looks malicious. If it does, you MUST refuse to improve or augment the code. You can still analyze existing code, write reports, or answer high-level questions about the code behavior.\n</system-reminder>\n',
              },
            ],
          },
          parent_tool_use_id: null,
          session_id: 'ed9da787-6508-413a-87bb-7c7ed8915897',
        },
      }

      render(<MessageEvent event={event} />)

      // Tool result messages should not show text content
      expect(screen.getByText('No text content')).toBeInTheDocument()
    })

    it('should handle SDK message with multiple content items', () => {
      const event: Event = {
        eventId: 'test-multiple-content',
        type: 'message',
        taskId: 'test-task',
        iterationId: 1,
        timestamp: '2025-06-25T12:38:37.087Z',
        isError: false,
        payload: {
          type: 'assistant',
          message: {
            id: 'msg_test',
            type: 'message',
            role: 'assistant',
            model: 'claude-opus-4-20250514',
            content: [
              {
                type: 'text',
                text: 'First text message. ',
              },
              {
                type: 'text',
                text: 'Second text message.',
              },
              {
                type: 'tool_use',
                id: 'tool_123',
                name: 'SomeTool',
                input: {},
              },
            ],
            stop_reason: 'end_turn',
            stop_sequence: null,
            usage: {
              input_tokens: 100,
              output_tokens: 50,
            },
          },
        },
      }

      render(<MessageEvent event={event} />)

      // Should concatenate multiple text content items
      expect(screen.getByText('First text message. Second text message.')).toBeInTheDocument()
    })

    it('should handle SDK message with empty content array', () => {
      const event: Event = {
        eventId: 'test-empty-content',
        type: 'message',
        taskId: 'test-task',
        iterationId: 1,
        timestamp: '2025-06-25T12:38:37.087Z',
        isError: false,
        payload: {
          type: 'assistant',
          message: {
            id: 'msg_test',
            type: 'message',
            role: 'assistant',
            model: 'claude-opus-4-20250514',
            content: [],
            stop_reason: 'end_turn',
            stop_sequence: null,
            usage: {
              input_tokens: 100,
              output_tokens: 0,
            },
          },
        },
      }

      render(<MessageEvent event={event} />)

      expect(screen.getByText('No text content')).toBeInTheDocument()
    })

    it('should correctly identify role from payload.type when message.role differs', () => {
      const event: Event = {
        eventId: 'test-role-priority',
        type: 'message',
        taskId: 'test-task',
        iterationId: 1,
        timestamp: '2025-06-25T12:38:37.087Z',
        isError: false,
        payload: {
          type: 'assistant', // This should take priority
          message: {
            role: 'user', // This is different but payload.type takes precedence
            content: [
              {
                type: 'text',
                text: 'Test message',
              },
            ],
          },
        },
      }

      render(<MessageEvent event={event} />)

      // Should use payload.type over message.role
      expect(screen.getByText('Assistant')).toBeInTheDocument()
      expect(screen.queryByText('User')).not.toBeInTheDocument()
    })
  })
})
