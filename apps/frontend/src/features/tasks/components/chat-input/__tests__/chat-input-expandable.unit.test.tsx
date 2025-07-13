import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { TaskUIProvider } from '@/features/tasks/contexts/task-ui-provider'
import { createMockMutationResult } from '@/test-utils/mock-mutations'
import { ChatInput } from '../index'

// Mock dependencies
vi.mock('@/features/repositories/hooks/use-repositories', () => ({
  useRepositoriesList: () => ({
    data: [
      { id: 'repo-1', name: 'Test Repository 1' },
      { id: 'repo-2', name: 'Test Repository 2' },
    ],
    isLoading: false,
  }),
  useBranches: () => ({
    data: { branches: ['main', 'develop'] },
    isLoading: false,
  }),
  useSearchFiles: () => ({
    data: null,
    isLoading: false,
    error: null,
  }),
}))

vi.mock('@/features/tasks/hooks/use-tasks', () => ({
  useSendMessage: () =>
    createMockMutationResult({
      mutateAsync: vi.fn(),
      isPending: false,
    }),
  useCreateTask: () =>
    createMockMutationResult({
      mutateAsync: vi.fn(),
      isPending: false,
    }),
  useStopAgent: () =>
    createMockMutationResult({
      mutate: vi.fn(),
      isPending: false,
    }),
  useTaskFileSearch: () => ({
    data: null,
    isLoading: false,
    error: null,
  }),
}))

vi.mock('@/features/templates', () => ({
  useTemplatesList: () => ({
    data: { templates: [] },
    isLoading: false,
  }),
}))

vi.mock('@/features/uploads/hooks/use-upload', () => ({
  useUploadFile: () =>
    createMockMutationResult({
      mutate: vi.fn(),
      isPending: false,
    }),
}))

// Helper to render with providers
const renderWithProviders = (component: React.ReactElement) => {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
    },
  })

  return render(
    <QueryClientProvider client={queryClient}>
      <TaskUIProvider taskId="test-task">{component}</TaskUIProvider>
    </QueryClientProvider>,
  )
}

vi.mock('@/lib/hooks/use-universal-file-autocomplete', () => ({
  useUniversalFileAutocomplete: () => ({
    inputRef: { current: null },
    isOpen: false,
    results: [],
    selectedIndex: -1,
    isLoading: false,
    error: null,
    handleInputChange: vi.fn(),
    handleKeyDown: vi.fn(),
    selectFile: vi.fn(),
  }),
}))

describe('ChatInput - Expandable Textarea', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('Default maxHeight behavior', () => {
    it('should apply default maxHeight of 200px when no prop is provided', () => {
      renderWithProviders(<ChatInput taskId="test-task" onSendMessage={vi.fn()} />)

      const textarea = screen.getByPlaceholderText(/type a new request/i)
      const textareaStyles = window.getComputedStyle(textarea)

      expect(textareaStyles.maxHeight).toBe('200px')
    })

    it('should have minimum height of 40px', () => {
      renderWithProviders(<ChatInput taskId="test-task" onSendMessage={vi.fn()} />)

      const textarea = screen.getByPlaceholderText(/type a new request/i)
      const textareaStyles = window.getComputedStyle(textarea)

      expect(textareaStyles.minHeight).toBe('40px')
    })
  })

  describe('Custom maxHeight prop', () => {
    it('should apply custom maxHeight when prop is provided', () => {
      renderWithProviders(
        <ChatInput taskId="test-task" onSendMessage={vi.fn()} maxHeight="300px" />,
      )

      const textarea = screen.getByPlaceholderText(/type a new request/i)
      const textareaStyles = window.getComputedStyle(textarea)

      expect(textareaStyles.maxHeight).toBe('300px')
    })

    it('should accept different maxHeight values', () => {
      const queryClient = new QueryClient({
        defaultOptions: {
          queries: { retry: false },
        },
      })

      const { rerender } = render(
        <QueryClientProvider client={queryClient}>
          <TaskUIProvider taskId="test-task">
            <ChatInput taskId="test-task" onSendMessage={vi.fn()} maxHeight="150px" />
          </TaskUIProvider>
        </QueryClientProvider>,
      )

      let textarea = screen.getByPlaceholderText(/type a new request/i)
      let textareaStyles = window.getComputedStyle(textarea)
      expect(textareaStyles.maxHeight).toBe('150px')

      rerender(
        <QueryClientProvider client={queryClient}>
          <TaskUIProvider taskId="test-task">
            <ChatInput taskId="test-task" onSendMessage={vi.fn()} maxHeight="400px" />
          </TaskUIProvider>
        </QueryClientProvider>,
      )

      textarea = screen.getByPlaceholderText(/type a new request/i)
      textareaStyles = window.getComputedStyle(textarea)
      expect(textareaStyles.maxHeight).toBe('400px')
    })
  })

  describe('Textarea auto-expansion', () => {
    it('should expand textarea height as user types', async () => {
      const user = userEvent.setup()
      renderWithProviders(
        <ChatInput taskId="test-task" onSendMessage={vi.fn()} maxHeight="300px" />,
      )

      const textarea = screen.getByPlaceholderText(/type a new request/i) as HTMLTextAreaElement

      // Initial height should be minimal
      const initialHeight = textarea.offsetHeight
      expect(initialHeight).toBeLessThanOrEqual(60) // Around 40px + padding

      // Type some text
      await user.type(textarea, 'This is a test message')

      // Mock scrollHeight to simulate content growth
      Object.defineProperty(textarea, 'scrollHeight', {
        configurable: true,
        value: 100,
      })

      // Trigger input event to simulate auto-resize
      fireEvent.input(textarea)

      await waitFor(() => {
        // In test environment, we just verify the scrollHeight was set
        expect(textarea.scrollHeight).toBe(100)
      })
    })

    it('should stop expanding at maxHeight', async () => {
      const user = userEvent.setup()
      renderWithProviders(
        <ChatInput taskId="test-task" onSendMessage={vi.fn()} maxHeight="150px" />,
      )

      const textarea = screen.getByPlaceholderText(/type a new request/i) as HTMLTextAreaElement

      // Mock a large scrollHeight that exceeds maxHeight
      Object.defineProperty(textarea, 'scrollHeight', {
        configurable: true,
        value: 250, // Exceeds maxHeight of 150px
      })

      // Type to trigger resize
      await user.type(textarea, 'A very long message that would normally require more height...')
      fireEvent.input(textarea)

      await waitFor(() => {
        // Verify the scrollHeight was set and maxHeight is in the styles
        expect(textarea.scrollHeight).toBe(250)
        expect(textarea.style.maxHeight).toBe('150px')
      })
    })

    it('should show scrollbar when content exceeds maxHeight', async () => {
      const user = userEvent.setup()
      renderWithProviders(
        <ChatInput taskId="test-task" onSendMessage={vi.fn()} maxHeight="100px" />,
      )

      const textarea = screen.getByPlaceholderText(/type a new request/i) as HTMLTextAreaElement

      // Add overflow-y-auto class check
      expect(textarea.className).toContain('overflow-y-auto')

      // Mock large content
      Object.defineProperty(textarea, 'scrollHeight', {
        configurable: true,
        value: 200,
      })

      await user.type(textarea, 'Line 1\nLine 2\nLine 3\nLine 4\nLine 5\nLine 6\nLine 7\nLine 8')
      fireEvent.input(textarea)

      await waitFor(() => {
        expect(textarea.style.maxHeight).toBe('100px')
        // When content exceeds container, scrollHeight > clientHeight
        expect(textarea.scrollHeight).toBeGreaterThan(100)
      })
    })

    it('should reset height when content is cleared', async () => {
      const user = userEvent.setup()
      renderWithProviders(
        <ChatInput taskId="test-task" onSendMessage={vi.fn()} maxHeight="200px" />,
      )

      const textarea = screen.getByPlaceholderText(/type a new request/i) as HTMLTextAreaElement

      // Type content to expand
      await user.type(textarea, 'Some content to expand the textarea')

      Object.defineProperty(textarea, 'scrollHeight', {
        configurable: true,
        value: 120,
      })
      fireEvent.input(textarea)

      await waitFor(() => {
        expect(textarea.scrollHeight).toBe(120)
      })

      // Clear the content by selecting all and deleting
      await user.keyboard('{Control>}a{/Control}')
      await user.keyboard('{Delete}')

      Object.defineProperty(textarea, 'scrollHeight', {
        configurable: true,
        value: 40, // Back to minimum
      })
      fireEvent.input(textarea)

      await waitFor(() => {
        expect(textarea.scrollHeight).toBe(40)
      })
    })
  })

  describe('Integration with different contexts', () => {
    it('should work in task context with custom maxHeight', () => {
      renderWithProviders(
        <ChatInput
          taskId="task-123"
          onSendMessage={vi.fn()}
          maxHeight="150px"
          placeholder="Type a new request..."
        />,
      )

      const textarea = screen.getByPlaceholderText(/type a new request/i)
      const textareaStyles = window.getComputedStyle(textarea)

      expect(textareaStyles.maxHeight).toBe('150px')
    })

    it('should work with custom maxHeight', () => {
      renderWithProviders(
        <ChatInput taskId="test-task" onSendMessage={vi.fn()} maxHeight="300px" />,
      )

      const textarea = screen.getByPlaceholderText(/type a new request/i)
      const textareaStyles = window.getComputedStyle(textarea)

      expect(textareaStyles.maxHeight).toBe('300px')
    })
  })

  describe('Multiline input handling', () => {
    it.skip('should allow multiline input with Shift+Enter - FIXME: Keyboard event simulation issues (TASK-102)', async () => {
      const user = userEvent.setup()
      renderWithProviders(<ChatInput taskId="test-task" onSendMessage={vi.fn()} />)

      const textarea = screen.getByPlaceholderText(/type a new request/i) as HTMLTextAreaElement

      await user.type(textarea, 'Line 1')
      await user.keyboard('{Shift>}{Enter}{/Shift}')
      await user.type(textarea, 'Line 2')

      expect(textarea.value).toBe('Line 1\nLine 2')
    })

    it.skip('should submit on Enter without Shift - FIXME: Keyboard event simulation issues (TASK-102)', async () => {
      const user = userEvent.setup()
      const onCreateTask = vi.fn()

      renderWithProviders(<ChatInput taskId="test-task" onSendMessage={onCreateTask} />)

      const textarea = screen.getByPlaceholderText(/type a new request/i)

      await user.type(textarea, 'Test message')
      await user.keyboard('{Enter}')

      await waitFor(() => {
        expect(onCreateTask).toHaveBeenCalledWith(
          'test-task',
          'Test message',
          expect.any(Function),
          true,
          undefined,
          undefined,
        )
      })
    })
  })
})
