import type { ToolCallDetailResponse, UiMessage } from '@moatless/api-client'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen } from '@/test-utils/render'
import { MessageHistory } from '../message-history'

// Mock the on-demand tool-call detail hook so the component tree stays
// independent of TanStack Query's network/cache layer in this unit suite.
const mockUseTaskToolCall = vi.hoisted(() =>
  vi.fn<
    (
      taskId: string,
      toolUseId: string | null | undefined,
      enabled: boolean,
    ) => { data: ToolCallDetailResponse | undefined; isLoading: boolean }
  >(),
)

vi.mock('@/features/tasks/hooks/use-tasks', () => ({
  useTaskToolCall: mockUseTaskToolCall,
}))

describe('MessageHistory - Tier 1 State Machine Tests', () => {
  beforeEach(() => {
    mockUseTaskToolCall.mockReset()
    mockUseTaskToolCall.mockReturnValue({ data: undefined, isLoading: false })
  })

  const createUserMessage = (id: string, content: string): UiMessage => ({
    uuid: id,
    role: 'user',
    content: [{ type: 'text', text: content }],
    toolResults: [],
    messageType: 'user',
    timestamp: new Date().toISOString(),
  })

  const createAssistantMessage = (id: string, content: string): UiMessage => ({
    uuid: id,
    role: 'assistant',
    content: [{ type: 'text', text: content }],
    toolResults: [],
    messageType: 'assistant',
    timestamp: new Date().toISOString(),
  })

  const createToolUseMessage = (id: string, toolName: string): UiMessage => ({
    uuid: id,
    role: 'assistant',
    content: [
      {
        type: 'tool_use',
        toolUseId: `tool-${id}`,
        toolName,
        toolInput: { test: 'data' },
      },
    ],
    toolResults: [],
    messageType: 'assistant',
    timestamp: new Date().toISOString(),
  })

  describe('Message Rendering', () => {
    it('renders user messages', () => {
      const messages: UiMessage[] = [createUserMessage('1', 'Hello')]

      render(<MessageHistory messages={messages} />)

      expect(screen.getByText('Hello')).toBeInTheDocument()
    })

    it('renders assistant messages', () => {
      const messages: UiMessage[] = [createAssistantMessage('1', 'Hi there')]

      render(<MessageHistory messages={messages} />)

      expect(screen.getByText('Hi there')).toBeInTheDocument()
    })

    it('renders multiple messages in order', () => {
      const messages: UiMessage[] = [
        createUserMessage('1', 'First message'),
        createAssistantMessage('2', 'Second message'),
        createUserMessage('3', 'Third message'),
      ]

      render(<MessageHistory messages={messages} />)

      const elements = screen.getAllByText(/message/)
      expect(elements).toHaveLength(3)
    })
  })

  describe('Pagination wiring', () => {
    it('renders messages flattened from multiple pages', () => {
      // Simulates `chat-container` flattening pages into a single chronological list.
      const olderPage = [
        createUserMessage('u-old', 'old user'),
        createAssistantMessage('a-old', 'old reply'),
      ]
      const newerPage = [createUserMessage('u-new', 'new user')]
      const messages: UiMessage[] = [...olderPage, ...newerPage]

      render(<MessageHistory messages={messages} hasPreviousPage isFetchingPreviousPage={false} />)

      expect(screen.getByText('old user')).toBeInTheDocument()
      expect(screen.getByText('old reply')).toBeInTheDocument()
      expect(screen.getByText('new user')).toBeInTheDocument()
    })

    it('shows older-pages spinner while fetching previous page', () => {
      render(
        <MessageHistory
          messages={[createUserMessage('1', 'Hello')]}
          hasPreviousPage
          isFetchingPreviousPage
        />,
      )

      expect(screen.getByText(/Loading older messages/i)).toBeInTheDocument()
    })

    it('renders one DOM node per renderable message', () => {
      const messages: UiMessage[] = [
        createUserMessage('u-1', 'one'),
        createAssistantMessage('a-1', 'two'),
        createUserMessage('u-2', 'three'),
      ]

      render(<MessageHistory messages={messages} />)

      expect(screen.getByText('one')).toBeInTheDocument()
      expect(screen.getByText('two')).toBeInTheDocument()
      expect(screen.getByText('three')).toBeInTheDocument()
    })
  })

  describe('Loading State', () => {
    it('does not show skeleton when messages exist', () => {
      const messages: UiMessage[] = [createUserMessage('1', 'Hello')]

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
      const messages: UiMessage[] = [createUserMessage('1', 'Hello')]

      render(<MessageHistory messages={messages} isRunning />)

      expect(screen.getByText(/Thinking/i)).toBeInTheDocument()
    })

    it('does not show thinking indicator when not running', () => {
      const messages: UiMessage[] = [createUserMessage('1', 'Hello')]

      render(<MessageHistory messages={messages} />)

      expect(screen.queryByText(/Thinking/i)).not.toBeInTheDocument()
    })
  })

  describe('Tool Call Expansion', () => {
    const createToolUseWithResultMessage = (
      id: string,
      toolName: string,
      preview: string,
      resultPreview: string,
    ): UiMessage => ({
      uuid: id,
      role: 'assistant',
      content: [
        {
          type: 'tool_use',
          toolUseId: `tool-${id}`,
          toolName,
          // Slim preview shape produced by the backend when summary=true.
          toolInput: { file_path: preview },
        },
      ],
      toolResults: [
        {
          toolUseId: `tool-${id}`,
          content: resultPreview,
          isError: false,
        },
      ],
      messageType: 'assistant',
      timestamp: new Date().toISOString(),
    })

    it('renders tool use messages', () => {
      const messages: UiMessage[] = [createToolUseMessage('1', 'Read')]

      const { container } = render(<MessageHistory messages={messages} />)

      expect(container).toBeInTheDocument()
    })

    it('fetches and renders full tool input + result when expanded', async () => {
      const user = userEvent.setup()
      const fullInput = { file_path: '/full/path.txt', extra: 'arg' }
      const fullResult = 'full file contents that exceed the slim preview'

      // Returns the loaded detail whenever the hook is enabled.
      mockUseTaskToolCall.mockImplementation((_taskId, _toolUseId, enabled) =>
        enabled
          ? {
              data: {
                toolUseId: 'tool-1',
                toolName: 'Read',
                toolInput: fullInput,
                resultContent: fullResult,
                isError: false,
              },
              isLoading: false,
            }
          : { data: undefined, isLoading: false },
      )

      const messages: UiMessage[] = [
        createToolUseWithResultMessage('1', 'Read', '/full/p…', 'truncated…'),
      ]

      render(<MessageHistory taskId="task-1" messages={messages} />)

      // Collapsed state: hook is enabled=false; collapsed row hides the result.
      expect(mockUseTaskToolCall).toHaveBeenCalledWith('task-1', 'tool-1', false)
      expect(screen.queryByText(fullResult)).not.toBeInTheDocument()

      // Click the tool-call header to expand.
      await user.click(screen.getByText('Read'))

      // Hook re-runs with enabled=true; full input + result render.
      expect(mockUseTaskToolCall).toHaveBeenCalledWith('task-1', 'tool-1', true)
      expect(screen.getByText(fullResult)).toBeInTheDocument()
      expect(screen.getByText(/\/full\/path\.txt/)).toBeInTheDocument()
    })
  })

  describe('Stick-to-bottom behaviour', () => {
    // Helper that overrides jsdom's flat scroll metrics on the live element
    // so we can drive the stick-to-bottom decision from a unit test. jsdom
    // does not compute layout so scrollHeight/clientHeight default to 0.
    const setScrollMetrics = (
      el: HTMLElement,
      metrics: { scrollHeight: number; clientHeight: number; scrollTop: number },
    ) => {
      Object.defineProperty(el, 'scrollHeight', {
        configurable: true,
        get: () => metrics.scrollHeight,
      })
      Object.defineProperty(el, 'clientHeight', {
        configurable: true,
        get: () => metrics.clientHeight,
      })
      Object.defineProperty(el, 'scrollTop', {
        configurable: true,
        get: () => metrics.scrollTop,
        set: (v: number) => {
          metrics.scrollTop = v
        },
      })
      return metrics
    }

    it('does not snap to bottom when content grows while user is scrolled away', () => {
      const initialMessages: UiMessage[] = [
        createUserMessage('1', 'msg one'),
        createAssistantMessage('2', 'msg two'),
      ]

      const { container, rerender } = render(<MessageHistory messages={initialMessages} />)

      const scrollContainer = container.querySelector('.overflow-y-auto') as HTMLDivElement | null
      expect(scrollContainer).not.toBeNull()
      if (!scrollContainer) {
        return
      }

      // User has scrolled near the top: 4000 px above the bottom — well past
      // AT_BOTTOM_THRESHOLD (50). Drive the scroll event so handleScroll
      // captures these into the prev-metric refs.
      const metrics = setScrollMetrics(scrollContainer, {
        scrollHeight: 5000,
        clientHeight: 900,
        scrollTop: 100,
      })
      fireEvent.scroll(scrollContainer)

      // SSE-driven update: page grows by one short message. Pre-commit
      // metrics still place the user 4000 px from bottom → snap MUST skip.
      metrics.scrollHeight = 5050

      rerender(
        <MessageHistory
          messages={[...initialMessages, createAssistantMessage('3', 'msg three')]}
        />,
      )

      // scrollTop unchanged. If snap had run, it would have written 5050.
      expect(metrics.scrollTop).toBe(100)
    })
  })

  describe('Custom className', () => {
    it('applies custom className', () => {
      const { container } = render(<MessageHistory messages={[]} className="custom-class" />)

      expect(container.firstChild).toHaveClass('custom-class')
    })
  })
})
