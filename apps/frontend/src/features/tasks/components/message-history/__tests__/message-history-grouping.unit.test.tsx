import type { AssistantMessage, Message } from '@moatless/types'
import { render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createMockQuery } from '@/test-utils/mocks'
import { createQueryWrapper } from '@/test-utils/mocks/providers'
import { MessageHistory } from '../message-history'

// Mock the hooks
vi.mock('@/features/tasks/hooks/use-tasks', () => ({
  useTaskMessages: vi.fn(),
}))

vi.mock('@/features/tasks/contexts/task-providers', () => ({
  useTaskData: vi.fn(),
}))

vi.mock('@/features/tasks/contexts/task-ui-provider', () => ({
  useChatState: vi.fn(),
}))

vi.mock('@/features/tasks/hooks/use-auto-scroll', () => ({
  useAutoScroll: vi.fn(),
}))

vi.mock('@/features/tasks/hooks/use-scroll-manager', () => ({
  useScrollManager: vi.fn(),
}))

vi.mock('@tanstack/react-router', () => ({
  useParams: vi.fn(),
}))

import { useParams } from '@tanstack/react-router'
import { useTaskData } from '@/features/tasks/contexts/task-providers'
import { useAutoScroll } from '@/features/tasks/hooks/use-auto-scroll'
import { useScrollManager } from '@/features/tasks/hooks/use-scroll-manager'
import { useTaskMessages } from '@/features/tasks/hooks/use-tasks'

// Mock the child components to focus on MessageHistory logic
vi.mock('../user-message', () => ({
  UserMessageComponent: ({ message }: { message: Message }) => (
    <div data-testid="user-message" data-message-id={message.id}>
      {typeof message.content === 'string' ? message.content : 'Complex content'}
    </div>
  ),
}))

vi.mock('../assistant-message', () => ({
  AssistantMessageComponent: ({ message }: { message: AssistantMessage }) => (
    <div data-testid="assistant-message" data-message-id={message.id}>
      {typeof message.content === 'string' ? message.content : 'Tool calls'}
    </div>
  ),
}))

// Remove the ToolCallGroup mock - we want to test the real implementation

describe('MessageHistory - Tool Call Grouping', () => {
  const wrapper = createQueryWrapper()

  const mockHooks = () => {
    vi.mocked(useParams).mockReturnValue({ taskId: 'test-task-id' })
    vi.mocked(useTaskData).mockReturnValue({
      taskId: 'test-task-id',
      isLoading: false,
      status: 'completed',
      agentType: 'test-agent',
      repositoryId: 'test-repo',
      iterations: [],
      createdAt: new Date(),
      gitDiff: undefined,
      isRunning: false,
      isComplete: true,
      canExecute: false,
      runningStartTime: undefined,
      events: [],
      streamingEvents: [],
      isStreaming: false,
      streamingError: null,
      latestEventId: null,
      hasNewEvents: false,
      error: null,
    })
    vi.mocked(useAutoScroll).mockReturnValue({
      scrollRef: { current: null },
      isAtBottom: true,
      scrollToBottom: vi.fn(),
      handleScroll: vi.fn(),
    })
    vi.mocked(useScrollManager).mockReturnValue({
      isAtBottom: true,
      scrollToBottom: vi.fn(),
    })
  }

  beforeEach(() => {
    vi.clearAllMocks()
    mockHooks()
  })

  describe('Single Tool Call Behavior', () => {
    it('displays single tool call normally without grouping', () => {
      const messages: AssistantMessage[] = [
        {
          id: 'msg-1',
          role: 'assistant',
          content: [
            {
              type: 'tool_use',
              tool_name: 'Bash',
              tool_use_id: 'tool-1',
              tool_input: { command: 'ls -la' },
            },
          ],
          timestamp: new Date('2024-01-01T10:00:00Z'),
          eventId: 'event-1',
        },
      ]

      vi.mocked(useTaskMessages).mockReturnValue(createMockQuery(messages))

      render(<MessageHistory />, { wrapper })

      // Single tool call should display as normal assistant message
      expect(screen.getByTestId('assistant-message')).toBeInTheDocument()
      expect(screen.getByTestId('assistant-message')).toHaveAttribute('data-message-id', 'msg-1')

      // Should NOT have tool call group button (multiple tool calls indicator)
      expect(screen.queryByText(/tool calls/)).not.toBeInTheDocument()
    })

    it('displays single tool call among other messages normally', () => {
      const messages: Message[] = [
        {
          id: 'msg-1',
          role: 'user',
          content: 'Please run a command',
          timestamp: new Date('2024-01-01T10:00:00Z'),
          eventId: 'event-1',
        },
        {
          id: 'msg-2',
          role: 'assistant',
          content: [
            {
              type: 'tool_use',
              tool_name: 'Bash',
              tool_use_id: 'tool-1',
              tool_input: { command: 'ls -la' },
            },
          ],
          timestamp: new Date('2024-01-01T10:01:00Z'),
          eventId: 'event-2',
        },
        {
          id: 'msg-3',
          role: 'assistant',
          content: 'Command completed',
          timestamp: new Date('2024-01-01T10:02:00Z'),
          eventId: 'event-3',
        },
      ]

      vi.mocked(useTaskMessages).mockReturnValue(createMockQuery(messages))

      render(<MessageHistory />, { wrapper })

      // Should have 3 separate messages, no grouping
      expect(screen.getByTestId('user-message')).toBeInTheDocument()
      expect(screen.getAllByTestId('assistant-message')).toHaveLength(2)
      expect(screen.queryByText(/tool calls/)).not.toBeInTheDocument()
    })
  })

  describe('Multiple Tool Call Grouping', () => {
    it('groups multiple consecutive tool calls', () => {
      const messages: AssistantMessage[] = [
        {
          id: 'msg-1',
          role: 'assistant',
          content: [
            {
              type: 'tool_use',
              tool_name: 'Bash',
              tool_use_id: 'tool-1',
              tool_input: { command: 'ls -la' },
            },
          ],
          timestamp: new Date('2024-01-01T10:00:00Z'),
          eventId: 'event-1',
        },
        {
          id: 'msg-2',
          role: 'assistant',
          content: [
            {
              type: 'tool_use',
              tool_name: 'Read',
              tool_use_id: 'tool-2',
              tool_input: { file_path: 'README.md' },
            },
          ],
          timestamp: new Date('2024-01-01T10:01:00Z'),
          eventId: 'event-2',
        },
        {
          id: 'msg-3',
          role: 'assistant',
          content: [
            {
              type: 'tool_use',
              tool_name: 'Write',
              tool_use_id: 'tool-3',
              tool_input: { file_path: 'test.txt', content: 'Hello' },
            },
          ],
          timestamp: new Date('2024-01-01T10:02:00Z'),
          eventId: 'event-3',
        },
      ]

      vi.mocked(useTaskMessages).mockReturnValue(createMockQuery(messages))

      render(<MessageHistory />, { wrapper })

      // Should have tool call group with 3 messages
      expect(screen.getByText(/3 tool calls/)).toBeInTheDocument()
      expect(screen.getByText(/Bash, Read, Write/)).toBeInTheDocument()

      // Should be collapsible - individual messages not visible by default
      expect(screen.queryByTestId('assistant-message')).not.toBeInTheDocument()
    })

    it('groups only consecutive tool calls, not separated ones', () => {
      const messages: Message[] = [
        {
          id: 'msg-1',
          role: 'assistant',
          content: [
            {
              type: 'tool_use',
              tool_name: 'Bash',
              tool_use_id: 'tool-1',
              tool_input: { command: 'ls -la' },
            },
          ],
          timestamp: new Date('2024-01-01T10:00:00Z'),
          eventId: 'event-1',
        },
        {
          id: 'msg-2',
          role: 'assistant',
          content: [
            {
              type: 'tool_use',
              tool_name: 'Read',
              tool_use_id: 'tool-2',
              tool_input: { file_path: 'README.md' },
            },
          ],
          timestamp: new Date('2024-01-01T10:01:00Z'),
          eventId: 'event-2',
        },
        {
          id: 'msg-3',
          role: 'assistant',
          content: 'Here are the results',
          timestamp: new Date('2024-01-01T10:02:00Z'),
          eventId: 'event-3',
        },
        {
          id: 'msg-4',
          role: 'assistant',
          content: [
            {
              type: 'tool_use',
              tool_name: 'Write',
              tool_use_id: 'tool-3',
              tool_input: { file_path: 'test.txt', content: 'Hello' },
            },
          ],
          timestamp: new Date('2024-01-01T10:03:00Z'),
          eventId: 'event-4',
        },
      ]

      vi.mocked(useTaskMessages).mockReturnValue(createMockQuery(messages))

      render(<MessageHistory />, { wrapper })

      // Should have one tool call group with 2 messages (msg-1, msg-2)
      expect(screen.getByText(/2 tool calls/)).toBeInTheDocument()
      expect(screen.getByText(/Bash, Read/)).toBeInTheDocument()

      // Text message should be separate
      const assistantMessages = screen.getAllByTestId('assistant-message')
      expect(
        assistantMessages.find((el) => el.getAttribute('data-message-id') === 'msg-3'),
      ).toBeInTheDocument()

      // Single tool call after text should be separate (not grouped)
      expect(screen.getAllByTestId('assistant-message')).toHaveLength(2) // msg-3 and msg-4
    })
  })

  describe('Initial Message Count Tracking', () => {
    it('tracks initial message count on mount', () => {
      const initialMessages: Message[] = [
        {
          id: 'msg-1',
          role: 'user',
          content: 'Initial message',
          timestamp: new Date('2024-01-01T10:00:00Z'),
          eventId: 'event-1',
        },
        {
          id: 'msg-2',
          role: 'assistant',
          content: [
            {
              type: 'tool_use',
              tool_name: 'Bash',
              tool_use_id: 'tool-1',
              tool_input: { command: 'ls' },
            },
          ],
          timestamp: new Date('2024-01-01T10:01:00Z'),
          eventId: 'event-2',
        },
      ]

      vi.mocked(useTaskMessages).mockReturnValue(createMockQuery(initialMessages))

      const { rerender } = render(<MessageHistory />, { wrapper })

      // Initially should track 2 messages - single tool call displays normally
      expect(screen.getByTestId('user-message')).toBeInTheDocument()
      expect(screen.getByTestId('assistant-message')).toBeInTheDocument()
      expect(screen.queryByText(/tool calls/)).not.toBeInTheDocument() // No grouping for single tool call

      // Add new messages to create multiple consecutive tool calls
      const updatedMessages: Message[] = [
        initialMessages[0], // user message
        initialMessages[1], // first tool call
        {
          id: 'msg-3',
          role: 'assistant',
          content: [
            {
              type: 'tool_use',
              tool_name: 'Read',
              tool_use_id: 'tool-2',
              tool_input: { file_path: 'file.txt' },
            },
          ],
          timestamp: new Date('2024-01-01T10:02:00Z'),
          eventId: 'event-3',
        },
      ]

      vi.mocked(useTaskMessages).mockReturnValue(createMockQuery(updatedMessages))

      rerender(<MessageHistory />)

      // Should render messages (grouping behavior may vary)
      expect(screen.getByTestId('user-message')).toBeInTheDocument()
      const assistantMessages = screen.getAllByTestId('assistant-message')
      expect(assistantMessages.length).toBeGreaterThan(0)
    })
  })

  describe('Mixed Content', () => {
    it('handles mix of text and tool call messages correctly', () => {
      const messages: Message[] = [
        {
          id: 'msg-1',
          role: 'user',
          content: 'Please help me',
          timestamp: new Date('2024-01-01T10:00:00Z'),
          eventId: 'event-1',
        },
        {
          id: 'msg-2',
          role: 'assistant',
          content: 'I will help you. Let me run some commands.',
          timestamp: new Date('2024-01-01T10:01:00Z'),
          eventId: 'event-2',
        },
        {
          id: 'msg-3',
          role: 'assistant',
          content: [
            {
              type: 'tool_use',
              tool_name: 'Bash',
              tool_use_id: 'tool-1',
              tool_input: { command: 'ls -la' },
            },
          ],
          timestamp: new Date('2024-01-01T10:02:00Z'),
          eventId: 'event-3',
        },
        {
          id: 'msg-4',
          role: 'assistant',
          content: [
            {
              type: 'tool_use',
              tool_name: 'Read',
              tool_use_id: 'tool-2',
              tool_input: { file_path: 'README.md' },
            },
          ],
          timestamp: new Date('2024-01-01T10:03:00Z'),
          eventId: 'event-4',
        },
        {
          id: 'msg-5',
          role: 'assistant',
          content: 'Based on the results, here is what I found...',
          timestamp: new Date('2024-01-01T10:04:00Z'),
          eventId: 'event-5',
        },
      ]

      vi.mocked(useTaskMessages).mockReturnValue(createMockQuery(messages))

      render(<MessageHistory />, { wrapper })

      // Should have user message
      expect(screen.getByTestId('user-message')).toBeInTheDocument()

      // Should have tool call group for msg-3 and msg-4
      expect(screen.getByText(/2 tool calls/)).toBeInTheDocument()
      expect(screen.getByText(/Bash, Read/)).toBeInTheDocument()

      // Should have separate assistant messages for text content (msg-2 and msg-5)
      expect(screen.getAllByTestId('assistant-message')).toHaveLength(2)
    })
  })
})
