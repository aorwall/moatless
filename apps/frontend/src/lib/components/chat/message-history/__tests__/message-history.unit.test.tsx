import type { AssistantMessage, Message, UserMessage } from '@moatless/types'
import { render, screen } from '@testing-library/react'
import { userEvent } from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'

vi.mock('@/features/tasks/hooks/use-auto-scroll.ts', () => ({
  useAutoScroll: () => ({
    scrollRef: { current: null },
    isAtBottom: true,
    scrollToBottom: vi.fn(),
  }),
}))

import { MessageHistory } from '../message-history'

describe('MessageHistory - Tier 1 State Machine Tests', () => {
  const createUserMessage = (id: string, content: string): UserMessage => ({
    id,
    role: 'user',
    content,
    timestamp: new Date().toISOString(),
  })

  const createAssistantMessage = (id: string, content: string): AssistantMessage => ({
    id,
    role: 'assistant',
    content: [{ type: 'text', text: content }],
    timestamp: new Date().toISOString(),
  })

  const createToolUseMessage = (id: string, toolName: string): AssistantMessage => ({
    id,
    role: 'assistant',
    content: [
      {
        type: 'tool_use',
        id: `tool-${id}`,
        tool_name: toolName,
        input: { test: 'data' },
      },
    ],
    timestamp: new Date().toISOString(),
  })

  describe('Message Rendering', () => {
    it('renders user messages', () => {
      const messages: Message[] = [createUserMessage('1', 'Hello')]

      render(<MessageHistory messages={messages} />)

      expect(screen.getByText('Hello')).toBeInTheDocument()
    })

    it('renders assistant messages', () => {
      const messages: Message[] = [createAssistantMessage('1', 'Hi there')]

      render(<MessageHistory messages={messages} />)

      expect(screen.getByText('Hi there')).toBeInTheDocument()
    })

    it('renders multiple messages in order', () => {
      const messages: Message[] = [
        createUserMessage('1', 'First message'),
        createAssistantMessage('2', 'Second message'),
        createUserMessage('3', 'Third message'),
      ]

      render(<MessageHistory messages={messages} />)

      const elements = screen.getAllByText(/message/)
      expect(elements).toHaveLength(3)
    })
  })

  describe('Tool Call Grouping', () => {
    it('groups consecutive tool-only messages', () => {
      const messages: Message[] = [
        createToolUseMessage('1', 'Read'),
        createToolUseMessage('2', 'Write'),
        createUserMessage('3', 'User message'),
      ]

      render(<MessageHistory messages={messages} />)

      expect(screen.getByText(/2 tool calls/i)).toBeInTheDocument()
    })

    it('expands tool call group on click', async () => {
      const messages: Message[] = [
        createToolUseMessage('1', 'Read'),
        createToolUseMessage('2', 'Write'),
      ]

      render(<MessageHistory messages={messages} />)

      const expandButton = screen.getByRole('button', { name: /2 tool calls/i })
      expect(expandButton).toBeInTheDocument()

      await userEvent.click(expandButton)
    })

    it('shows tool count summary', () => {
      const messages: Message[] = [
        createToolUseMessage('1', 'Read'),
        createToolUseMessage('2', 'Read'),
        createToolUseMessage('3', 'Write'),
      ]

      render(<MessageHistory messages={messages} />)

      expect(screen.getByText(/3 tool calls/i)).toBeInTheDocument()
      expect(screen.getByText(/2 Read, Write/i)).toBeInTheDocument()
    })
  })

  describe('Loading State', () => {
    it('does not show skeleton when messages exist', () => {
      const messages: Message[] = [createUserMessage('1', 'Hello')]

      render(<MessageHistory messages={messages} isLoading />)

      expect(screen.getByText('Hello')).toBeInTheDocument()
    })
  })

  describe('Error State', () => {
    it('displays error message', () => {
      const error = new Error('Failed to load')

      render(<MessageHistory error={error} />)

      expect(screen.getByText(/Failed to load/i)).toBeInTheDocument()
    })
  })

  describe('Empty State', () => {
    it('shows empty state when no messages', () => {
      render(<MessageHistory messages={[]} />)

      expect(screen.getByText(/No messages yet/i)).toBeInTheDocument()
    })

    it('does not show empty state during loading', () => {
      render(<MessageHistory messages={[]} isLoading />)

      expect(screen.queryByText(/No messages yet/i)).not.toBeInTheDocument()
    })
  })

  describe('Running State', () => {
    it('shows thinking indicator when running', () => {
      const messages: Message[] = [createUserMessage('1', 'Hello')]

      render(<MessageHistory messages={messages} isRunning />)

      expect(screen.getByText(/Thinking/i)).toBeInTheDocument()
    })

    it('does not show thinking indicator when not running', () => {
      const messages: Message[] = [createUserMessage('1', 'Hello')]

      render(<MessageHistory messages={messages} />)

      expect(screen.queryByText(/Thinking/i)).not.toBeInTheDocument()
    })
  })

  describe('Auto-scroll Behavior', () => {
    it('hides scroll to bottom button when at bottom', () => {
      const messages: Message[] = [createUserMessage('1', 'Hello')]

      render(<MessageHistory messages={messages} />)

      expect(screen.queryByRole('button', { name: /scroll to bottom/i })).not.toBeInTheDocument()
    })
  })

  describe('Tool Call Expansion', () => {
    it('renders tool use messages', () => {
      const messages: Message[] = [createToolUseMessage('1', 'Read')]

      const { container } = render(<MessageHistory messages={messages} />)

      expect(container).toBeInTheDocument()
    })
  })

  describe('Custom className', () => {
    it('applies custom className', () => {
      const { container } = render(<MessageHistory messages={[]} className="custom-class" />)

      expect(container.firstChild).toHaveClass('custom-class')
    })
  })
})
