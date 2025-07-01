import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
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
}))

vi.mock('@/features/tasks/hooks/use-tasks', () => ({
  useSendMessage: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
  }),
  useCreateTask: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
  }),
}))

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
      render(<ChatInput />)

      const textarea = screen.getByPlaceholderText(/describe your coding task/i)
      const textareaStyles = window.getComputedStyle(textarea)

      expect(textareaStyles.maxHeight).toBe('200px')
    })

    it('should have minimum height of 40px', () => {
      render(<ChatInput />)

      const textarea = screen.getByPlaceholderText(/describe your coding task/i)
      const textareaStyles = window.getComputedStyle(textarea)

      expect(textareaStyles.minHeight).toBe('40px')
    })
  })

  describe('Custom maxHeight prop', () => {
    it('should apply custom maxHeight when prop is provided', () => {
      render(<ChatInput maxHeight="300px" />)

      const textarea = screen.getByPlaceholderText(/describe your coding task/i)
      const textareaStyles = window.getComputedStyle(textarea)

      expect(textareaStyles.maxHeight).toBe('300px')
    })

    it('should accept different maxHeight values', () => {
      const { rerender } = render(<ChatInput maxHeight="150px" />)

      let textarea = screen.getByPlaceholderText(/describe your coding task/i)
      let textareaStyles = window.getComputedStyle(textarea)
      expect(textareaStyles.maxHeight).toBe('150px')

      rerender(<ChatInput maxHeight="400px" />)

      textarea = screen.getByPlaceholderText(/describe your coding task/i)
      textareaStyles = window.getComputedStyle(textarea)
      expect(textareaStyles.maxHeight).toBe('400px')
    })
  })

  describe('Textarea auto-expansion', () => {
    it('should expand textarea height as user types', async () => {
      const user = userEvent.setup()
      render(<ChatInput maxHeight="300px" />)

      const textarea = screen.getByPlaceholderText(
        /describe your coding task/i,
      ) as HTMLTextAreaElement

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
        expect(textarea.style.height).toBe('100px')
      })
    })

    it('should stop expanding at maxHeight', async () => {
      const user = userEvent.setup()
      render(<ChatInput maxHeight="150px" />)

      const textarea = screen.getByPlaceholderText(
        /describe your coding task/i,
      ) as HTMLTextAreaElement

      // Mock a large scrollHeight that exceeds maxHeight
      Object.defineProperty(textarea, 'scrollHeight', {
        configurable: true,
        value: 250, // Exceeds maxHeight of 150px
      })

      // Type to trigger resize
      await user.type(textarea, 'A very long message that would normally require more height...')
      fireEvent.input(textarea)

      await waitFor(() => {
        // Height should be capped at maxHeight
        expect(textarea.style.height).toBe('150px')
      })
    })

    it('should show scrollbar when content exceeds maxHeight', async () => {
      const user = userEvent.setup()
      render(<ChatInput maxHeight="100px" />)

      const textarea = screen.getByPlaceholderText(
        /describe your coding task/i,
      ) as HTMLTextAreaElement

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
        expect(textarea.style.height).toBe('100px')
        // When content exceeds container, scrollHeight > clientHeight
        expect(textarea.scrollHeight).toBeGreaterThan(100)
      })
    })

    it('should reset height when content is cleared', async () => {
      const user = userEvent.setup()
      render(<ChatInput maxHeight="200px" />)

      const textarea = screen.getByPlaceholderText(
        /describe your coding task/i,
      ) as HTMLTextAreaElement

      // Type content to expand
      await user.type(textarea, 'Some content to expand the textarea')

      Object.defineProperty(textarea, 'scrollHeight', {
        configurable: true,
        value: 120,
      })
      fireEvent.input(textarea)

      await waitFor(() => {
        expect(textarea.style.height).toBe('120px')
      })

      // Clear the content
      await user.clear(textarea)

      Object.defineProperty(textarea, 'scrollHeight', {
        configurable: true,
        value: 40, // Back to minimum
      })
      fireEvent.input(textarea)

      await waitFor(() => {
        expect(textarea.style.height).toBe('40px')
      })
    })
  })

  describe('Integration with different contexts', () => {
    it('should work in task context with custom maxHeight', () => {
      render(<ChatInput taskId="task-123" maxHeight="150px" placeholder="Type a new request..." />)

      const textarea = screen.getByPlaceholderText(/type a new request/i)
      const textareaStyles = window.getComputedStyle(textarea)

      expect(textareaStyles.maxHeight).toBe('150px')
    })

    it('should work in homepage context with custom maxHeight', () => {
      render(<ChatInput showEnvironmentSelector={true} showTitle={true} maxHeight="300px" />)

      const textarea = screen.getByPlaceholderText(/describe your coding task/i)
      const textareaStyles = window.getComputedStyle(textarea)

      expect(textareaStyles.maxHeight).toBe('300px')
      expect(screen.getByText(/what should we build today/i)).toBeInTheDocument()
    })
  })

  describe('Multiline input handling', () => {
    it('should allow multiline input with Shift+Enter', async () => {
      const user = userEvent.setup()
      render(<ChatInput />)

      const textarea = screen.getByPlaceholderText(
        /describe your coding task/i,
      ) as HTMLTextAreaElement

      await user.type(textarea, 'Line 1')
      await user.keyboard('{Shift>}{Enter}{/Shift}')
      await user.type(textarea, 'Line 2')

      expect(textarea.value).toBe('Line 1\nLine 2')
    })

    it('should submit on Enter without Shift', async () => {
      const user = userEvent.setup()
      const onCreateTask = vi.fn()

      render(<ChatInput showEnvironmentSelector={true} onCreateTask={onCreateTask} />)

      const textarea = screen.getByPlaceholderText(/describe your coding task/i)

      await user.type(textarea, 'Test message')
      await user.keyboard('{Enter}')

      await waitFor(() => {
        expect(onCreateTask).toHaveBeenCalledWith(
          'Test message',
          expect.any(String),
          expect.any(Function),
          expect.any(String),
        )
      })
    })
  })
})
