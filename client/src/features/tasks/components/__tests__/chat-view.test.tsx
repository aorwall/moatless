import { fireEvent, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'

import {
  createMockTask,
  createMockTaskIteration,
  createMockTaskWithIterations,
} from '@/test-utils/factories'
import { renderWithRouter } from '@/test-utils/render'

import { useTaskEventsWithStreaming } from '../../hooks/use-task-events'
import { useTask, useTaskWorkspaceStatus } from '../../hooks/use-tasks'
import { useTaskUIStore } from '../../stores/task-ui-store'
import { ChatView } from '../chat-view'

// Mock dependencies
vi.mock('../../hooks/use-tasks')
vi.mock('../../hooks/use-task-events')
vi.mock('../../stores/task-ui-store')
vi.mock('@tanstack/react-router', () => ({
  useParams: () => ({ taskId: 'task-123' }),
}))

// Mock ReactMarkdown to simplify testing
vi.mock('react-markdown', () => ({
  default: ({ children }: { children: string }) => <div>{children}</div>,
}))

// Mock ScrollArea
vi.mock('@/lib/components/ui/scroll-area', () => ({
  ScrollArea: ({ children, ...props }: { children: React.ReactNode; [key: string]: unknown }) => (
    <div {...props}>{children}</div>
  ),
}))

// Mock ChatInput
vi.mock('../chat-input', () => ({
  ChatInput: ({ className }: { className?: string }) => (
    <div data-testid="task-chat-input" className={className}>
      Mock Chat Input
    </div>
  ),
}))

// Mock ErrorBoundary
vi.mock('@/lib/components/error-boundary', () => ({
  ErrorBoundary: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}))

// Mock DOM methods
Object.defineProperty(window, 'scrollIntoView', {
  value: vi.fn(),
  writable: true,
})

beforeAll(() => {
  Element.prototype.scrollIntoView = vi.fn()
})

describe('ChatView', () => {
  const mockSetSelectedIteration = vi.fn()

  beforeEach(() => {
    vi.clearAllMocks()

    // Mock store with proper Zustand mocking
    vi.mocked(useTaskUIStore).mockReturnValue(mockSetSelectedIteration)

    // Default mock for useTask
    vi.mocked(useTask).mockReturnValue({
      data: createMockTaskWithIterations(),
      isLoading: false,
      error: null,
      refetch: vi.fn(),
    })

    // Default mock for useTaskWorkspaceStatus
    vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
      data: null,
      isLoading: false,
      error: null,
      refetch: vi.fn(),
    })

    // Default mock for useTaskEventsWithStreaming
    vi.mocked(useTaskEventsWithStreaming).mockReturnValue({
      events: [],
      isStreaming: false,
      isLoading: false,
      error: null,
    })
  })

  describe('Rendering', () => {
    it('renders task iterations', () => {
      const task = createMockTaskWithIterations({}, 2)
      vi.mocked(useTask).mockReturnValue({
        data: task,
        isLoading: false,
        error: null,
        refetch: vi.fn(),
      })

      renderWithRouter(<ChatView />)

      // Check that both iterations are rendered
      expect(screen.getByText(task.iterations[0].initial_message)).toBeInTheDocument()
      expect(screen.getByText(task.iterations[1].initial_message)).toBeInTheDocument()
    })

    it('renders iteration metadata', () => {
      const iteration = createMockTaskIteration({
        duration_ms: 125000, // 2m 5s
        message_count: 10,
        cost_usd: 0.0123,
      })
      const task = createMockTask({ iterations: [iteration] })

      vi.mocked(useTask).mockReturnValue({
        data: task,
        isLoading: false,
        error: null,
        refetch: vi.fn(),
      })

      renderWithRouter(<ChatView />)

      expect(screen.getByText('2m 5s')).toBeInTheDocument()
      expect(screen.getByText('10 messages')).toBeInTheDocument()
      expect(screen.getByText('$0.0123')).toBeInTheDocument()
    })

    it('renders result message when present', () => {
      const iteration = createMockTaskIteration({
        result: 'Task completed successfully',
      })
      const task = createMockTask({ iterations: [iteration] })

      vi.mocked(useTask).mockReturnValue({
        data: task,
        isLoading: false,
        error: null,
        refetch: vi.fn(),
      })

      renderWithRouter(<ChatView />)

      expect(screen.getByText('Task completed successfully')).toBeInTheDocument()
    })

    it('renders error message when present', () => {
      const iteration = createMockTaskIteration({
        error_message: 'Task failed with error',
      })
      const task = createMockTask({ iterations: [iteration] })

      vi.mocked(useTask).mockReturnValue({
        data: task,
        isLoading: false,
        error: null,
        refetch: vi.fn(),
      })

      renderWithRouter(<ChatView />)

      expect(screen.getByText('Task failed with error')).toBeInTheDocument()
    })

    it('shows processing indicator when task is running', () => {
      const task = createMockTaskWithIterations({ is_running: true }, 1)

      vi.mocked(useTask).mockReturnValue({
        data: task,
        isLoading: false,
        error: null,
        refetch: vi.fn(),
      })

      renderWithRouter(<ChatView />)

      expect(screen.getByText('Processing...')).toBeInTheDocument()
    })

    it('does not show processing indicator when task is not running', () => {
      const task = createMockTaskWithIterations({ is_running: false }, 1)

      vi.mocked(useTask).mockReturnValue({
        data: task,
        isLoading: false,
        error: null,
        refetch: vi.fn(),
      })

      renderWithRouter(<ChatView />)

      expect(screen.queryByText('Processing...')).not.toBeInTheDocument()
    })

    it('returns null when no metadata', () => {
      vi.mocked(useTask).mockReturnValue({
        data: undefined,
        isLoading: false,
        error: null,
        refetch: vi.fn(),
      })

      const { container } = renderWithRouter(<ChatView />)

      expect(container.firstChild).toBeNull()
    })
  })

  describe('Interactions', () => {
    it('handles iteration click', async () => {
      const user = userEvent.setup()
      const iteration = createMockTaskIteration({ iteration: 1 })
      const task = createMockTask({ iterations: [iteration] })

      vi.mocked(useTask).mockReturnValue({
        data: task,
        isLoading: false,
        error: null,
        refetch: vi.fn(),
      })

      renderWithRouter(<ChatView />)

      const button = screen.getByRole('button', {
        name: /click to view events/i,
      })
      await user.click(button)

      expect(mockSetSelectedIteration).toHaveBeenCalledWith(iteration)
    })

    it('handles keyboard navigation with Enter', () => {
      const iteration = createMockTaskIteration({ iteration: 1 })
      const task = createMockTask({ iterations: [iteration] })

      vi.mocked(useTask).mockReturnValue({
        data: task,
        isLoading: false,
        error: null,
        refetch: vi.fn(),
      })

      renderWithRouter(<ChatView />)

      const button = screen.getByRole('button', {
        name: /click to view events/i,
      })
      fireEvent.keyDown(button, { key: 'Enter' })

      expect(mockSetSelectedIteration).toHaveBeenCalledWith(iteration)
    })

    it('handles keyboard navigation with Space', () => {
      const iteration = createMockTaskIteration({ iteration: 1 })
      const task = createMockTask({ iterations: [iteration] })

      vi.mocked(useTask).mockReturnValue({
        data: task,
        isLoading: false,
        error: null,
        refetch: vi.fn(),
      })

      renderWithRouter(<ChatView />)

      const button = screen.getByRole('button', {
        name: /click to view events/i,
      })
      fireEvent.keyDown(button, { key: ' ' })

      expect(mockSetSelectedIteration).toHaveBeenCalledWith(iteration)
    })

    it('ignores other keys', () => {
      const iteration = createMockTaskIteration({ iteration: 1 })
      const task = createMockTask({ iterations: [iteration] })

      vi.mocked(useTask).mockReturnValue({
        data: task,
        isLoading: false,
        error: null,
        refetch: vi.fn(),
      })

      renderWithRouter(<ChatView />)

      const button = screen.getByRole('button', {
        name: /click to view events/i,
      })
      fireEvent.keyDown(button, { key: 'a' })

      expect(mockSetSelectedIteration).not.toHaveBeenCalled()
    })
  })

  describe('Formatting', () => {
    it('formats duration correctly', () => {
      const testCases = [
        { ms: 0, expected: '0m 0s' },
        { ms: 1000, expected: '0m 1s' },
        { ms: 60000, expected: '1m 0s' },
        { ms: 65000, expected: '1m 5s' },
        { ms: 125000, expected: '2m 5s' },
      ]

      testCases.forEach(({ ms, expected }) => {
        const iteration = createMockTaskIteration({ duration_ms: ms })
        const task = createMockTask({ iterations: [iteration] })

        vi.mocked(useTask).mockReturnValue({
          data: task,
          isLoading: false,
          error: null,
          refetch: vi.fn(),
        })

        const { rerender } = renderWithRouter(<ChatView />)
        expect(screen.getByText(expected)).toBeInTheDocument()
        rerender(<ChatView />)
      })
    })

    it('formats cost correctly', () => {
      const testCases = [
        { cost: 0, expected: '$0.0000' },
        { cost: 0.1, expected: '$0.1000' },
        { cost: 0.0123, expected: '$0.0123' },
        { cost: 1.23456, expected: '$1.2346' }, // Should round
      ]

      testCases.forEach(({ cost, expected }) => {
        const iteration = createMockTaskIteration({ cost_usd: cost })
        const task = createMockTask({ iterations: [iteration] })

        vi.mocked(useTask).mockReturnValue({
          data: task,
          isLoading: false,
          error: null,
          refetch: vi.fn(),
        })

        const { rerender } = renderWithRouter(<ChatView />)
        expect(screen.getByText(expected)).toBeInTheDocument()
        rerender(<ChatView />)
      })
    })
  })

  describe('Scrolling Behavior', () => {
    it('scrolls to bottom when task is running', () => {
      const scrollIntoViewMock = vi.fn()
      Element.prototype.scrollIntoView = scrollIntoViewMock

      const task = createMockTaskWithIterations({ is_running: true }, 1)

      vi.mocked(useTask).mockReturnValue({
        data: task,
        isLoading: false,
        error: null,
        refetch: vi.fn(),
      })

      renderWithRouter(<ChatView />)

      expect(scrollIntoViewMock).toHaveBeenCalledWith({ behavior: 'smooth' })
    })

    it('scrolls to bottom on mount regardless of task state', () => {
      const scrollIntoViewMock = vi.fn()
      Element.prototype.scrollIntoView = scrollIntoViewMock

      const task = createMockTaskWithIterations({ is_running: false }, 1)

      vi.mocked(useTask).mockReturnValue({
        data: task,
        isLoading: false,
        error: null,
        refetch: vi.fn(),
      })

      renderWithRouter(<ChatView />)

      // Component always scrolls to bottom on mount
      expect(scrollIntoViewMock).toHaveBeenCalledWith({ behavior: 'smooth' })
    })
  })

  describe('Edge Cases', () => {
    it('handles task with no iterations', () => {
      const task = createMockTask({ iterations: [] })

      vi.mocked(useTask).mockReturnValue({
        data: task,
        isLoading: false,
        error: null,
        refetch: vi.fn(),
      })

      renderWithRouter(<ChatView />)

      // Should render without errors
      expect(screen.queryByRole('button')).not.toBeInTheDocument()
    })

    it('handles iterations without iteration_id', () => {
      const iteration1 = createMockTaskIteration({ iteration_id: undefined })
      const iteration2 = createMockTaskIteration({ iteration_id: undefined })
      const task = createMockTask({ iterations: [iteration1, iteration2] })

      vi.mocked(useTask).mockReturnValue({
        data: task,
        isLoading: false,
        error: null,
        refetch: vi.fn(),
      })

      renderWithRouter(<ChatView />)

      // Should render both iterations using index as key
      const buttons = screen.getAllByRole('button')
      expect(buttons).toHaveLength(2)
    })

    it('does not show processing indicator when no iterations', () => {
      const task = createMockTask({ is_running: true, iterations: [] })

      vi.mocked(useTask).mockReturnValue({
        data: task,
        isLoading: false,
        error: null,
        refetch: vi.fn(),
      })

      renderWithRouter(<ChatView />)

      expect(screen.queryByText('Processing...')).not.toBeInTheDocument()
    })
  })
})
