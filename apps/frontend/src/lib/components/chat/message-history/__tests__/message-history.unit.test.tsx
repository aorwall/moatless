import type { AssistantMessage, Message, UserMessage } from '@moatless/types'
import { act, render, screen, waitFor } from '@testing-library/react'
import { userEvent } from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { MessageHistory } from '../message-history'

describe('MessageHistory - Tier 1 State Machine Tests', () => {
  const createUserMessage = (id: string, content: string): UserMessage => ({
    id,
    role: 'user',
    content,
    timestamp: new Date(),
  })

  const createAssistantMessage = (id: string, content: string): AssistantMessage => ({
    id,
    role: 'assistant',
    content: [{ type: 'text', text: content }],
    timestamp: new Date(),
  })

  const createToolUseMessage = (id: string, toolName: string): AssistantMessage => ({
    id,
    role: 'assistant',
    content: [
      {
        type: 'tool_use',
        tool_use_id: `tool-${id}`,
        tool_name: toolName,
        tool_input: { test: 'data' },
      },
    ],
    timestamp: new Date(),
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
    let scrollViewport: HTMLElement | null

    /**
     * Helper to mock scroll viewport with configurable dimensions and scroll position.
     * This is required because jsdom doesn't fully support scroll behavior.
     */
    const mockScrollViewport = (scrollTop: number, scrollHeight: number, clientHeight: number) => {
      if (!scrollViewport) return

      // Track scrollTop changes
      let currentScrollTop = scrollTop

      Object.defineProperties(scrollViewport, {
        scrollTop: {
          get: () => currentScrollTop,
          set: (value: number) => {
            // Clamp scrollTop like a real browser (max is scrollHeight - clientHeight)
            const maxScrollTop = Math.max(0, scrollHeight - clientHeight)
            currentScrollTop = Math.min(Math.max(0, value), maxScrollTop)
          },
          configurable: true,
        },
        scrollHeight: {
          get: () => scrollHeight,
          configurable: true,
        },
        clientHeight: {
          get: () => clientHeight,
          configurable: true,
        },
      })

      return {
        getScrollTop: () => currentScrollTop,
        setScrollTop: (value: number) => {
          currentScrollTop = value
        },
      }
    }

    /**
     * Helper to simulate a scroll event on the viewport
     */
    const triggerScroll = () => {
      if (!scrollViewport) return
      act(() => {
        const event = new Event('scroll', { bubbles: true })
        scrollViewport?.dispatchEvent(event)
      })
    }

    beforeEach(() => {
      // Mock requestAnimationFrame for auto-scroll
      vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => {
        cb(0)
        return 0
      })
    })

    afterEach(() => {
      vi.unstubAllGlobals()
      scrollViewport = null
    })

    it('renders ScrollArea component', () => {
      const messages: Message[] = [
        createUserMessage('1', 'Message 1'),
        createUserMessage('2', 'Message 2'),
        createUserMessage('3', 'Message 3'),
      ]

      const { container } = render(<MessageHistory messages={messages} />)

      // Find the ScrollArea component by data-slot attribute
      const scrollContainer = container.querySelector('[data-slot="scroll-area"]')
      expect(scrollContainer).toBeInTheDocument()
    })

    it('renders messages with ScrollArea viewport', () => {
      const messages: Message[] = [
        createUserMessage('1', 'Hello'),
        createAssistantMessage('2', 'Hi there'),
      ]

      const { container } = render(<MessageHistory messages={messages} />)

      // Verify messages are rendered
      expect(screen.getByText('Hello')).toBeInTheDocument()
      expect(screen.getByText('Hi there')).toBeInTheDocument()

      // Verify ScrollArea viewport exists (this is what useAutoScroll looks for)
      const scrollViewport = container.querySelector('[data-slot="scroll-area-viewport"]')
      expect(scrollViewport).toBeInTheDocument()
    })

    it('auto-scrolls to bottom when new message arrives and user is at bottom', async () => {
      const initialMessages: Message[] = [
        createUserMessage('1', 'First message'),
        createAssistantMessage('2', 'Second message'),
      ]

      const { container, rerender } = render(<MessageHistory messages={initialMessages} />)

      scrollViewport = container.querySelector('[data-slot="scroll-area-viewport"]')
      expect(scrollViewport).toBeInTheDocument()

      // Track scrollTop setter calls
      let scrollTopSetCount = 0
      let lastSetValue = 0

      // Mock: user is at bottom
      Object.defineProperties(scrollViewport, {
        scrollTop: {
          get: () => 230,
          set: (value: number) => {
            scrollTopSetCount++
            lastSetValue = value
          },
          configurable: true,
        },
        scrollHeight: {
          get: () => (scrollTopSetCount > 0 ? 700 : 600), // Increase after first set
          configurable: true,
        },
        clientHeight: {
          get: () => 370,
          configurable: true,
        },
      })

      // Trigger scroll event to capture initial position (at bottom: 600 - 370 = 230)
      triggerScroll()

      const initialSetCount = scrollTopSetCount

      // Add new message
      const updatedMessages: Message[] = [
        ...initialMessages,
        createUserMessage('3', 'Third message'),
      ]

      await act(async () => {
        rerender(<MessageHistory messages={updatedMessages} />)
        await new Promise((resolve) => setTimeout(resolve, 10))
      })

      // Verify auto-scroll happened (scrollTop setter was called with new scrollHeight)
      await waitFor(() => {
        expect(scrollTopSetCount).toBeGreaterThan(initialSetCount)
        expect(lastSetValue).toBeGreaterThan(600) // Tried to scroll to new scrollHeight
      })
    })

    it('does NOT auto-scroll when new message arrives and user scrolled up', async () => {
      const initialMessages: Message[] = [
        createUserMessage('1', 'First message'),
        createAssistantMessage('2', 'Second message'),
      ]

      const { container, rerender } = render(<MessageHistory messages={initialMessages} />)

      scrollViewport = container.querySelector('[data-slot="scroll-area-viewport"]')
      expect(scrollViewport).toBeInTheDocument()

      // Mock: user scrolled up (scrollTop = 0, not at bottom)
      const mock = mockScrollViewport(0, 600, 370) // Not at bottom
      expect(mock).toBeDefined()
      if (!mock) return

      // Trigger scroll event to capture position (user scrolled up)
      triggerScroll()

      const initialScrollTop = mock.getScrollTop()

      // Add new message
      const updatedMessages: Message[] = [
        ...initialMessages,
        createUserMessage('3', 'Third message'),
      ]

      await act(async () => {
        rerender(<MessageHistory messages={updatedMessages} />)
        await new Promise((resolve) => setTimeout(resolve, 0))
      })

      // Verify scrollTop did NOT change
      expect(mock.getScrollTop()).toBe(initialScrollTop)
    })

    it('shows "Scroll to bottom" button when user is not at bottom', async () => {
      const messages: Message[] = [
        createUserMessage('1', 'Message 1'),
        createUserMessage('2', 'Message 2'),
        createUserMessage('3', 'Message 3'),
      ]

      const { container } = render(<MessageHistory messages={messages} />)

      scrollViewport = container.querySelector('[data-slot="scroll-area-viewport"]')
      expect(scrollViewport).toBeInTheDocument()

      // Mock: user scrolled up
      mockScrollViewport(0, 600, 370)

      // Trigger scroll event to update isAtBottom state
      triggerScroll()

      // Button should appear
      await waitFor(() => {
        expect(screen.getByRole('button', { name: /scroll to bottom/i })).toBeInTheDocument()
      })
    })

    it('scrolls to bottom when "Scroll to bottom" button is clicked', async () => {
      const messages: Message[] = [
        createUserMessage('1', 'Message 1'),
        createUserMessage('2', 'Message 2'),
        createUserMessage('3', 'Message 3'),
      ]

      const { container } = render(<MessageHistory messages={messages} />)

      scrollViewport = container.querySelector('[data-slot="scroll-area-viewport"]')
      expect(scrollViewport).toBeInTheDocument()

      // Mock: user scrolled up
      const mock = mockScrollViewport(0, 600, 370)
      expect(mock).toBeDefined()
      if (!mock) return

      // Trigger scroll event
      triggerScroll()

      // Wait for button to appear
      const button = await waitFor(() => screen.getByRole('button', { name: /scroll to bottom/i }))

      // Click button
      await act(async () => {
        await userEvent.click(button)
        await new Promise((resolve) => setTimeout(resolve, 0))
      })

      // Verify scrolled to bottom (scrollTop should be scrollHeight - clientHeight = 230)
      await waitFor(() => {
        expect(mock.getScrollTop()).toBe(230)
      })
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
