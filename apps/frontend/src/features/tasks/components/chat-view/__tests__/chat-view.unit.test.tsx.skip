import type { Message } from '@moatless/types'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createMockMutation, createMockQuery, createQueryWrapper } from '@/test-utils/mocks'
import { ChatView } from '../chat-view'

// Mock the child components
vi.mock('../../message-history/message-history', () => ({
  MessageHistory: ({ messages }: { messages: Message[] }) => (
    <div data-testid="message-history">{messages.length} messages</div>
  ),
}))

vi.mock('../../chat-input/chat-input', () => ({
  ChatInput: ({
    onSendMessage,
    isLoading,
    showCommands,
  }: {
    onSendMessage: (message: string) => void
    isLoading: boolean
    showCommands: boolean
  }) => (
    <div data-testid="chat-input">
      <input
        data-testid="message-input"
        placeholder="Type a message..."
        onChange={(_e) => {
          // Simulate input change
        }}
      />
      <button
        data-testid="send-button"
        onClick={() => onSendMessage('test message')}
        disabled={isLoading}
      >
        {isLoading ? 'Sending...' : 'Send'}
      </button>
      {showCommands && <div data-testid="command-buttons">Commands</div>}
    </div>
  ),
}))

// Mock the API hooks
vi.mock('@/features/tasks/hooks/use-tasks', () => ({
  useTaskMessages: vi.fn(),
  useSendTaskMessage: vi.fn(),
}))

import { useSendTaskMessage, useTaskMessages } from '@/features/tasks/hooks/use-tasks'

const mockUseTaskMessages = vi.mocked(useTaskMessages)
const mockUseSendTaskMessage = vi.mocked(useSendTaskMessage)

describe('ChatView - Unit Tests', () => {
  const { wrapper } = createQueryWrapper()
  const taskId = 'test-task-1'
  let mockSendMessage: any

  beforeEach(() => {
    vi.clearAllMocks()

    mockSendMessage = vi.fn()

    // Default mock implementations
    mockUseTaskMessages.mockReturnValue(createMockQuery([]))
    mockUseSendTaskMessage.mockReturnValue(createMockMutation({ mutate: mockSendMessage }))
  })

  describe('Rendering', () => {
    it('renders with default props', () => {
      render(<ChatView taskId={taskId} />, { wrapper })

      expect(screen.getByTestId('message-history')).toBeInTheDocument()
      expect(screen.getByTestId('chat-input')).toBeInTheDocument()
    })

    it('passes messages to MessageHistory component', () => {
      const mockMessages: Message[] = [
        {
          id: 'msg-1',
          role: 'user',
          content: 'Hello',
          timestamp: '2024-01-01T10:00:00Z',
          toolCalls: [],
        },
        {
          id: 'msg-2',
          role: 'assistant',
          content: 'Hi there',
          timestamp: '2024-01-01T10:01:00Z',
          toolCalls: [],
        },
      ]

      mockUseTaskMessages.mockReturnValue(createMockQuery(mockMessages))

      render(<ChatView taskId={taskId} />, { wrapper })

      expect(screen.getByTestId('message-history')).toHaveTextContent('2 messages')
    })

    it('shows command buttons by default', () => {
      render(<ChatView taskId={taskId} />, { wrapper })

      expect(screen.getByTestId('command-buttons')).toBeInTheDocument()
    })

    it('hides command buttons when showCommands is false', () => {
      render(<ChatView taskId={taskId} showCommands={false} />, { wrapper })

      expect(screen.queryByTestId('command-buttons')).not.toBeInTheDocument()
    })

    it('applies custom className', () => {
      const customClass = 'custom-chat-view'
      const { container } = render(<ChatView taskId={taskId} className={customClass} />, {
        wrapper,
      })

      expect(container.firstChild).toHaveClass(customClass)
    })
  })

  describe('Message sending', () => {
    it('calls sendMessage when chat input submits', async () => {
      const user = userEvent.setup()

      render(<ChatView taskId={taskId} />, { wrapper })

      const sendButton = screen.getByTestId('send-button')
      await user.click(sendButton)

      expect(mockSendMessage).toHaveBeenCalledOnce()
      expect(mockSendMessage).toHaveBeenCalledWith({
        taskId,
        message: 'test message',
      })
    })

    it('handles empty message gracefully', async () => {
      const user = userEvent.setup()

      // Mock ChatInput to send empty message
      vi.mocked(require('../../chat-input/chat-input')).ChatInput.mockImplementation(
        ({ onSendMessage }: any) => (
          <button data-testid="send-empty-button" onClick={() => onSendMessage('')}>
            Send Empty
          </button>
        ),
      )

      render(<ChatView taskId={taskId} />, { wrapper })

      const sendButton = screen.getByTestId('send-empty-button')
      await user.click(sendButton)

      expect(mockSendMessage).toHaveBeenCalledWith({
        taskId,
        message: '',
      })
    })

    it('handles long messages', async () => {
      const user = userEvent.setup()
      const longMessage = 'a'.repeat(1000)

      vi.mocked(require('../../chat-input/chat-input')).ChatInput.mockImplementation(
        ({ onSendMessage }: any) => (
          <button data-testid="send-long-button" onClick={() => onSendMessage(longMessage)}>
            Send Long
          </button>
        ),
      )

      render(<ChatView taskId={taskId} />, { wrapper })

      const sendButton = screen.getByTestId('send-long-button')
      await user.click(sendButton)

      expect(mockSendMessage).toHaveBeenCalledWith({
        taskId,
        message: longMessage,
      })
    })
  })

  describe('Loading states', () => {
    it('shows loading state when messages are loading', () => {
      mockUseTaskMessages.mockReturnValue(createMockQuery([], { isLoading: true }))

      render(<ChatView taskId={taskId} />, { wrapper })

      // Should still render components but they should handle loading state
      expect(screen.getByTestId('message-history')).toBeInTheDocument()
      expect(screen.getByTestId('chat-input')).toBeInTheDocument()
    })

    it('shows loading state when sending message', () => {
      mockUseSendTaskMessage.mockReturnValue(
        createMockMutation({
          mutate: mockSendMessage,
          isPending: true,
        }),
      )

      render(<ChatView taskId={taskId} />, { wrapper })

      const sendButton = screen.getByTestId('send-button')
      expect(sendButton).toBeDisabled()
      expect(sendButton).toHaveTextContent('Sending...')
    })
  })

  describe('Error handling', () => {
    it('handles message loading error gracefully', () => {
      mockUseTaskMessages.mockReturnValue(
        createMockQuery([], {
          isError: true,
          error: new Error('Failed to load messages'),
        }),
      )

      render(<ChatView taskId={taskId} />, { wrapper })

      // Component should still render - error handling is done by parent components
      expect(screen.getByTestId('message-history')).toBeInTheDocument()
      expect(screen.getByTestId('chat-input')).toBeInTheDocument()
    })

    it('handles send message error gracefully', () => {
      mockUseSendTaskMessage.mockReturnValue(
        createMockMutation({
          mutate: mockSendMessage,
          isError: true,
          error: new Error('Failed to send message'),
        }),
      )

      render(<ChatView taskId={taskId} />, { wrapper })

      // Component should still be functional
      expect(screen.getByTestId('send-button')).toBeInTheDocument()
      expect(screen.getByTestId('send-button')).not.toBeDisabled()
    })
  })

  describe('Props validation', () => {
    it('requires taskId prop', () => {
      // @ts-expect-error Testing required prop
      expect(() => render(<ChatView />, { wrapper })).not.toThrow()
    })

    it('handles undefined taskId gracefully', () => {
      // @ts-expect-error Testing runtime safety
      render(<ChatView taskId={undefined} />, { wrapper })

      expect(screen.getByTestId('message-history')).toBeInTheDocument()
      expect(screen.getByTestId('chat-input')).toBeInTheDocument()
    })
  })

  describe('Integration behavior', () => {
    it('updates when new messages arrive', () => {
      const { rerender } = render(<ChatView taskId={taskId} />, { wrapper })

      expect(screen.getByTestId('message-history')).toHaveTextContent('0 messages')

      // Simulate new messages arriving
      const newMessages: Message[] = [
        {
          id: 'msg-1',
          role: 'user',
          content: 'New message',
          timestamp: '2024-01-01T10:00:00Z',
          toolCalls: [],
        },
      ]

      mockUseTaskMessages.mockReturnValue(createMockQuery(newMessages))
      rerender(<ChatView taskId={taskId} />)

      expect(screen.getByTestId('message-history')).toHaveTextContent('1 messages')
    })

    it('maintains component structure during state changes', () => {
      const { rerender } = render(<ChatView taskId={taskId} />, { wrapper })

      // Initial state
      expect(screen.getByTestId('message-history')).toBeInTheDocument()
      expect(screen.getByTestId('chat-input')).toBeInTheDocument()

      // Loading state
      mockUseTaskMessages.mockReturnValue(createMockQuery([], { isLoading: true }))
      rerender(<ChatView taskId={taskId} />)

      expect(screen.getByTestId('message-history')).toBeInTheDocument()
      expect(screen.getByTestId('chat-input')).toBeInTheDocument()

      // Error state
      mockUseTaskMessages.mockReturnValue(
        createMockQuery([], {
          isError: true,
          error: new Error('Test error'),
        }),
      )
      rerender(<ChatView taskId={taskId} />)

      expect(screen.getByTestId('message-history')).toBeInTheDocument()
      expect(screen.getByTestId('chat-input')).toBeInTheDocument()
    })
  })

  describe('Accessibility', () => {
    it('has proper structure for screen readers', () => {
      render(<ChatView taskId={taskId} />, { wrapper })

      const chatContainer = screen.getByRole('region', { name: /chat conversation/i })
      expect(chatContainer).toBeInTheDocument()
    })

    it('maintains focus management', async () => {
      const user = userEvent.setup()

      render(<ChatView taskId={taskId} />, { wrapper })

      const messageInput = screen.getByTestId('message-input')
      await user.click(messageInput)

      expect(messageInput).toHaveFocus()
    })
  })
})
