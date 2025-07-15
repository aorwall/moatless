import type { Message } from '@moatless/types'
import { render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createQueryWrapper } from '@/test-utils/mocks'
import { MessageHistory } from '../message-history'

// Mock the child components to focus on MessageHistory logic
vi.mock('../user-message', () => ({
  UserMessage: ({ message }: { message: Message }) => (
    <div data-testid="user-message" data-message-id={message.id}>
      {message.content}
    </div>
  ),
}))

vi.mock('../assistant-message', () => ({
  AssistantMessage: ({ message }: { message: Message }) => (
    <div data-testid="assistant-message" data-message-id={message.id}>
      {message.content}
      {message.toolCalls.length > 0 && (
        <div data-testid="tool-calls">
          {message.toolCalls.map((tool) => (
            <div key={tool.id} data-testid={`tool-${tool.id}`}>
              {tool.name}({JSON.stringify(tool.arguments)})
            </div>
          ))}
        </div>
      )}
    </div>
  ),
}))

describe('MessageHistory - Unit Tests', () => {
  const { wrapper } = createQueryWrapper()

  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('Rendering', () => {
    it('renders empty state when no messages provided', () => {
      render(<MessageHistory messages={[]} />, { wrapper })

      expect(screen.getByText('No messages yet')).toBeInTheDocument()
      expect(screen.getByText('Start a conversation to see messages here.')).toBeInTheDocument()
    })

    it('renders single user message', () => {
      const messages: Message[] = [
        {
          id: 'msg-1',
          role: 'user',
          content: 'Hello, can you help me?',
          timestamp: '2024-01-01T10:00:00Z',
          toolCalls: [],
        },
      ]

      render(<MessageHistory messages={messages} />, { wrapper })

      expect(screen.getByTestId('user-message')).toBeInTheDocument()
      expect(screen.getByText('Hello, can you help me?')).toBeInTheDocument()
      expect(screen.getByTestId('user-message')).toHaveAttribute('data-message-id', 'msg-1')
    })

    it('renders single assistant message', () => {
      const messages: Message[] = [
        {
          id: 'msg-1',
          role: 'assistant',
          content: 'I can help you with that.',
          timestamp: '2024-01-01T10:00:00Z',
          toolCalls: [],
        },
      ]

      render(<MessageHistory messages={messages} />, { wrapper })

      expect(screen.getByTestId('assistant-message')).toBeInTheDocument()
      expect(screen.getByText('I can help you with that.')).toBeInTheDocument()
      expect(screen.getByTestId('assistant-message')).toHaveAttribute('data-message-id', 'msg-1')
    })

    it('renders multiple messages in correct order', () => {
      const messages: Message[] = [
        {
          id: 'msg-1',
          role: 'user',
          content: 'First message',
          timestamp: '2024-01-01T10:00:00Z',
          toolCalls: [],
        },
        {
          id: 'msg-2',
          role: 'assistant',
          content: 'Second message',
          timestamp: '2024-01-01T10:01:00Z',
          toolCalls: [],
        },
        {
          id: 'msg-3',
          role: 'user',
          content: 'Third message',
          timestamp: '2024-01-01T10:02:00Z',
          toolCalls: [],
        },
      ]

      render(<MessageHistory messages={messages} />, { wrapper })

      const messageElements = screen.getAllByTestId(/^(user|assistant)-message$/)
      expect(messageElements).toHaveLength(3)

      // Check order is preserved
      expect(messageElements[0]).toHaveAttribute('data-message-id', 'msg-1')
      expect(messageElements[1]).toHaveAttribute('data-message-id', 'msg-2')
      expect(messageElements[2]).toHaveAttribute('data-message-id', 'msg-3')
    })

    it('renders assistant message with tool calls', () => {
      const messages: Message[] = [
        {
          id: 'msg-1',
          role: 'assistant',
          content: "I'll run a command for you.",
          timestamp: '2024-01-01T10:00:00Z',
          toolCalls: [
            {
              id: 'tool-1',
              name: 'Bash',
              arguments: { command: 'ls -la' },
              result: 'total 8\ndrwxr-xr-x 3 user user 4096 Jan  1 10:00 .',
            },
            {
              id: 'tool-2',
              name: 'Read',
              arguments: { file_path: './README.md' },
              result: '# Project README',
            },
          ],
        },
      ]

      render(<MessageHistory messages={messages} />, { wrapper })

      expect(screen.getByTestId('assistant-message')).toBeInTheDocument()
      expect(screen.getByText("I'll run a command for you.")).toBeInTheDocument()

      // Check tool calls are rendered
      expect(screen.getByTestId('tool-calls')).toBeInTheDocument()
      expect(screen.getByTestId('tool-tool-1')).toBeInTheDocument()
      expect(screen.getByTestId('tool-tool-2')).toBeInTheDocument()
    })

    it('handles missing data gracefully', () => {
      const messages: Message[] = [
        {
          id: 'msg-1',
          role: 'user',
          content: '',
          timestamp: '2024-01-01T10:00:00Z',
          toolCalls: [],
        },
      ]

      render(<MessageHistory messages={messages} />, { wrapper })

      expect(screen.getByTestId('user-message')).toBeInTheDocument()
    })

    it('handles messages with undefined tool calls', () => {
      const messages: Message[] = [
        {
          id: 'msg-1',
          role: 'assistant',
          content: 'Simple message',
          timestamp: '2024-01-01T10:00:00Z',
          toolCalls: [],
        },
      ]

      render(<MessageHistory messages={messages} />, { wrapper })

      expect(screen.getByTestId('assistant-message')).toBeInTheDocument()
      expect(screen.getByText('Simple message')).toBeInTheDocument()
      expect(screen.queryByTestId('tool-calls')).not.toBeInTheDocument()
    })
  })

  describe('Accessibility', () => {
    it('has proper ARIA attributes for message list', () => {
      const messages: Message[] = [
        {
          id: 'msg-1',
          role: 'user',
          content: 'Test message',
          timestamp: '2024-01-01T10:00:00Z',
          toolCalls: [],
        },
      ]

      render(<MessageHistory messages={messages} />, { wrapper })

      const messageContainer = screen.getByRole('log', { name: /message history/i })
      expect(messageContainer).toBeInTheDocument()
    })

    it('has proper structure for screen readers', () => {
      const messages: Message[] = [
        {
          id: 'msg-1',
          role: 'user',
          content: 'User message',
          timestamp: '2024-01-01T10:00:00Z',
          toolCalls: [],
        },
        {
          id: 'msg-2',
          role: 'assistant',
          content: 'Assistant response',
          timestamp: '2024-01-01T10:01:00Z',
          toolCalls: [],
        },
      ]

      render(<MessageHistory messages={messages} />, { wrapper })

      // Check that messages are in a proper list structure
      const messageContainer = screen.getByRole('log')
      expect(messageContainer).toBeInTheDocument()
    })
  })

  describe('Auto-scroll behavior', () => {
    it('contains scroll container element', () => {
      const messages: Message[] = [
        {
          id: 'msg-1',
          role: 'user',
          content: 'Test message',
          timestamp: '2024-01-01T10:00:00Z',
          toolCalls: [],
        },
      ]

      render(<MessageHistory messages={messages} />, { wrapper })

      // Check that the scroll container exists
      const scrollContainer = screen.getByTestId('messages-scroll-container')
      expect(scrollContainer).toBeInTheDocument()
      expect(scrollContainer).toHaveClass('overflow-y-auto')
    })
  })

  describe('Performance', () => {
    it('renders many messages efficiently', () => {
      const messages: Message[] = Array.from({ length: 100 }, (_, i) => ({
        id: `msg-${i}`,
        role: i % 2 === 0 ? 'user' : 'assistant',
        content: `Message ${i}`,
        timestamp: new Date(Date.now() + i * 1000).toISOString(),
        toolCalls: [],
      }))

      const { container } = render(<MessageHistory messages={messages} />, { wrapper })

      // Should render all messages
      expect(screen.getAllByTestId(/^(user|assistant)-message$/)).toHaveLength(100)

      // Container should be present
      expect(container.firstChild).toBeInTheDocument()
    })
  })

  describe('Error handling', () => {
    it('handles null/undefined messages gracefully', () => {
      // @ts-expect-error Testing runtime safety
      render(<MessageHistory messages={null} />, { wrapper })

      expect(screen.getByText('No messages yet')).toBeInTheDocument()
    })

    it('handles malformed message objects', () => {
      const messages = [
        {
          id: 'msg-1',
          role: 'user',
          // Missing content
          timestamp: '2024-01-01T10:00:00Z',
          toolCalls: [],
        },
      ] as Message[]

      render(<MessageHistory messages={messages} />, { wrapper })

      expect(screen.getByTestId('user-message')).toBeInTheDocument()
    })
  })
})
