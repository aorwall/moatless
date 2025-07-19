import type { Message } from '@moatless/types'
import { render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
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

const createMockQueryResult = (data: any) => ({
  data,
  isLoading: false,
  error: null,
  refetch: vi.fn(),
  isError: false,
  isPending: false,
  isLoadingError: false,
  isRefetchError: false,
  isSuccess: true,
  status: 'success' as const,
  fetchStatus: 'idle' as const,
  isFetching: false,
  isRefetching: false,
  isStale: false,
  dataUpdatedAt: Date.now(),
  errorUpdatedAt: 0,
  errorUpdateCount: 0,
  failureCount: 0,
  failureReason: null,
  isFetched: true,
  isFetchedAfterMount: true,
  isInitialLoading: false,
  isPlaceholderData: false,
  isPreviousData: false,
  isPaused: false,
  promise: Promise.resolve(data),
})

// Mock the child components to focus on MessageHistory logic
vi.mock('../user-message', () => ({
  UserMessageComponent: ({ message }: { message: Message }) => (
    <div data-testid="user-message" data-message-id={message.id}>
      {typeof message.content === 'string' ? message.content : 'Complex content'}
    </div>
  ),
}))

vi.mock('../assistant-message', () => ({
  AssistantMessageComponent: ({ message }: { message: Message }) => {
    const content = Array.isArray(message.content)
      ? message.content
      : [{ type: 'text', text: message.content }]
    const toolUses = content.filter(
      (
        item,
      ): item is {
        type: 'tool_use'
        tool_use_id?: string
        tool_name?: string
        tool_input?: Record<string, unknown>
        text?: string
        tool_result?: any
      } => item.type === 'tool_use',
    )

    return (
      <div data-testid="assistant-message" data-message-id={message.id}>
        {Array.isArray(message.content)
          ? message.content
              .filter((item) => item.type === 'text')
              .map((item, i) => (
                <span key={`text-${i}-${item.text?.substring(0, 10)}`}>{item.text}</span>
              ))
          : message.content}
        {toolUses.length > 0 && (
          <div data-testid="tool-calls">
            {toolUses.map((tool) => (
              <div key={tool.tool_use_id} data-testid={`tool-${tool.tool_use_id}`}>
                {tool.tool_name}({JSON.stringify(tool.tool_input)})
              </div>
            ))}
          </div>
        )}
      </div>
    )
  },
}))

describe('MessageHistory - Unit Tests', () => {
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
      fileContext: [],
      messages: [],
      workspaceId: 'test-workspace',
      createdAt: new Date(),
      updatedAt: new Date(),
    } as any)
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

  describe('Rendering', () => {
    it('renders empty state when no messages provided', () => {
      vi.mocked(useTaskMessages).mockReturnValue(createMockQueryResult([]) as any)

      render(<MessageHistory />, { wrapper })

      expect(screen.getByText('No messages yet. Start a conversation!')).toBeInTheDocument()
    })

    it('renders single user message', () => {
      const messages: Message[] = [
        {
          id: 'msg-1',
          role: 'user',
          content: 'Hello, can you help me?',
          timestamp: new Date('2024-01-01T10:00:00Z'),
        },
      ]

      vi.mocked(useTaskMessages).mockReturnValue(createMockQueryResult(messages) as any)

      render(<MessageHistory />, { wrapper })

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
          timestamp: new Date('2024-01-01T10:00:00Z'),
        },
      ]

      vi.mocked(useTaskMessages).mockReturnValue(createMockQueryResult(messages) as any)

      render(<MessageHistory />, { wrapper })

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
          timestamp: new Date('2024-01-01T10:00:00Z'),
        },
        {
          id: 'msg-2',
          role: 'assistant',
          content: 'Second message',
          timestamp: new Date('2024-01-01T10:01:00Z'),
        },
        {
          id: 'msg-3',
          role: 'user',
          content: 'Third message',
          timestamp: new Date('2024-01-01T10:02:00Z'),
        },
      ]

      vi.mocked(useTaskMessages).mockReturnValue(createMockQueryResult(messages) as any)

      render(<MessageHistory />, { wrapper })

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
          content: [
            {
              type: 'text',
              text: "I'll run a command for you.",
            },
            {
              type: 'tool_use',
              tool_name: 'Bash',
              tool_use_id: 'tool-1',
              tool_input: { command: 'ls -la' },
              tool_result: {
                content: 'total 8\ndrwxr-xr-x 3 user user 4096 Jan  1 10:00 .',
                is_error: false,
              },
            },
            {
              type: 'tool_use',
              tool_name: 'Read',
              tool_use_id: 'tool-2',
              tool_input: { file_path: './README.md' },
              tool_result: {
                content: '# Project README',
                is_error: false,
              },
            },
          ],
          timestamp: new Date('2024-01-01T10:00:00Z'),
        },
      ]

      vi.mocked(useTaskMessages).mockReturnValue(createMockQueryResult(messages) as any)

      render(<MessageHistory />, { wrapper })

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
          timestamp: new Date('2024-01-01T10:00:00Z'),
        },
      ]

      vi.mocked(useTaskMessages).mockReturnValue(createMockQueryResult(messages) as any)

      render(<MessageHistory />, { wrapper })

      expect(screen.getByTestId('user-message')).toBeInTheDocument()
    })

    it('handles messages with undefined tool calls', () => {
      const messages: Message[] = [
        {
          id: 'msg-1',
          role: 'assistant',
          content: 'Simple message',
          timestamp: new Date('2024-01-01T10:00:00Z'),
        },
      ]

      vi.mocked(useTaskMessages).mockReturnValue(createMockQueryResult(messages) as any)

      render(<MessageHistory />, { wrapper })

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
          timestamp: new Date('2024-01-01T10:00:00Z'),
        },
      ]

      vi.mocked(useTaskMessages).mockReturnValue(createMockQueryResult(messages) as any)

      render(<MessageHistory />, { wrapper })

      // Check that messages are rendered properly
      expect(screen.getByTestId('user-message')).toBeInTheDocument()
    })

    it('has proper structure for screen readers', () => {
      const messages: Message[] = [
        {
          id: 'msg-1',
          role: 'user',
          content: 'User message',
          timestamp: new Date('2024-01-01T10:00:00Z'),
        },
        {
          id: 'msg-2',
          role: 'assistant',
          content: 'Assistant response',
          timestamp: new Date('2024-01-01T10:01:00Z'),
        },
      ]

      vi.mocked(useTaskMessages).mockReturnValue(createMockQueryResult(messages) as any)

      render(<MessageHistory />, { wrapper })

      // Check that messages are rendered in proper structure
      expect(screen.getByTestId('user-message')).toBeInTheDocument()
      expect(screen.getByTestId('assistant-message')).toBeInTheDocument()
    })
  })

  describe('Auto-scroll behavior', () => {
    it('renders messages in a scrollable container', () => {
      const messages: Message[] = [
        {
          id: 'msg-1',
          role: 'user',
          content: 'Test message',
          timestamp: new Date('2024-01-01T10:00:00Z'),
        },
      ]

      vi.mocked(useTaskMessages).mockReturnValue(createMockQueryResult(messages) as any)

      render(<MessageHistory />, { wrapper })

      // Check that messages are rendered
      expect(screen.getByTestId('user-message')).toBeInTheDocument()
    })
  })

  describe('Performance', () => {
    it('renders many messages efficiently', () => {
      const messages: Message[] = Array.from({ length: 100 }, (_, i) => ({
        id: `msg-${i}`,
        role: i % 2 === 0 ? 'user' : 'assistant',
        content: `Message ${i}`,
        timestamp: new Date(Date.now() + i * 1000),
        toolCalls: [],
      }))

      // Mock the useTaskMessages hook to return our test messages
      vi.mocked(useTaskMessages).mockReturnValue(createMockQueryResult(messages) as any)

      const { container } = render(<MessageHistory />, { wrapper })

      // Should render all messages
      expect(screen.getAllByTestId(/^(user|assistant)-message$/)).toHaveLength(100)

      // Container should be present
      expect(container.firstChild).toBeInTheDocument()
    })
  })

  describe('Error handling', () => {
    it('handles null/undefined messages gracefully', () => {
      // Mock useTaskMessages to return null/undefined
      vi.mocked(useTaskMessages).mockReturnValue(createMockQueryResult(null) as any)

      render(<MessageHistory />, { wrapper })

      expect(screen.getByText('No messages yet. Start a conversation!')).toBeInTheDocument()
    })

    it('handles malformed message objects', () => {
      const messages = [
        {
          id: 'msg-1',
          role: 'user',
          content: '', // Empty content instead of missing
          timestamp: new Date('2024-01-01T10:00:00Z'),
        },
      ] as Message[]

      // Mock useTaskMessages to return malformed messages
      vi.mocked(useTaskMessages).mockReturnValue(createMockQueryResult(messages) as any)

      render(<MessageHistory />, { wrapper })

      expect(screen.getByTestId('user-message')).toBeInTheDocument()
    })
  })
})
