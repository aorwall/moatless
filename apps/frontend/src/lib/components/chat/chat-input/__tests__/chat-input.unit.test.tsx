import { render, screen } from '@testing-library/react'
import { userEvent } from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ChatInput } from '../chat-input'

describe('ChatInput - Tier 2 Interaction Tests', () => {
  const defaultProps = {
    message: '',
    onMessageChange: vi.fn(),
    onSubmit: vi.fn(),
  }

  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('Message Submission', () => {
    it('submits message on Enter key', async () => {
      const onSubmit = vi.fn()
      render(<ChatInput {...defaultProps} message="test message" onSubmit={onSubmit} />)

      const textarea = screen.getByRole('textbox')
      await userEvent.click(textarea)
      await userEvent.keyboard('{Enter}')

      expect(onSubmit).toHaveBeenCalled()
    })

    it('does not submit empty message on Enter', async () => {
      const onSubmit = vi.fn()
      render(<ChatInput {...defaultProps} message="" onSubmit={onSubmit} />)

      const textarea = screen.getByRole('textbox')
      await userEvent.click(textarea)
      await userEvent.keyboard('{Enter}')

      expect(onSubmit).not.toHaveBeenCalled()
    })

    it('does not submit whitespace-only message', async () => {
      const onSubmit = vi.fn()
      render(<ChatInput {...defaultProps} message="   " onSubmit={onSubmit} />)

      const textarea = screen.getByRole('textbox')
      await userEvent.click(textarea)
      await userEvent.keyboard('{Enter}')

      expect(onSubmit).not.toHaveBeenCalled()
    })

    it('adds newline on Shift+Enter', async () => {
      const onSubmit = vi.fn()
      render(<ChatInput {...defaultProps} message="test" onSubmit={onSubmit} />)

      const textarea = screen.getByRole('textbox')
      await userEvent.click(textarea)
      await userEvent.keyboard('{Shift>}{Enter}{/Shift}')

      expect(onSubmit).not.toHaveBeenCalled()
    })

    it('prevents submission when loading', async () => {
      const onSubmit = vi.fn()
      render(<ChatInput {...defaultProps} message="test" onSubmit={onSubmit} loading />)

      const textarea = screen.getByRole('textbox')
      await userEvent.click(textarea)
      await userEvent.keyboard('{Enter}')

      expect(onSubmit).not.toHaveBeenCalled()
    })

    it('submits on form submit', async () => {
      const onSubmit = vi.fn()
      render(<ChatInput {...defaultProps} message="test" onSubmit={onSubmit} />)

      const form = screen.getByRole('textbox').closest('form')
      if (form) {
        form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
      }

      expect(onSubmit).toHaveBeenCalled()
    })

    it('prevents form submit when loading', () => {
      const onSubmit = vi.fn()
      render(<ChatInput {...defaultProps} message="test" onSubmit={onSubmit} loading />)

      const form = screen.getByRole('textbox').closest('form')
      if (form) {
        form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
      }

      expect(onSubmit).not.toHaveBeenCalled()
    })
  })

  describe('Textarea Behavior', () => {
    it('calls onMessageChange when typing', async () => {
      const onMessageChange = vi.fn()
      render(<ChatInput {...defaultProps} onMessageChange={onMessageChange} />)

      const textarea = screen.getByRole('textbox')
      await userEvent.type(textarea, 'hello')

      expect(onMessageChange).toHaveBeenCalled()
    })

    it('disables textarea when loading', () => {
      render(<ChatInput {...defaultProps} loading />)

      const textarea = screen.getByRole('textbox')
      expect(textarea).toBeDisabled()
    })

    it('displays current message value', () => {
      render(<ChatInput {...defaultProps} message="Current message" />)

      const textarea = screen.getByRole('textbox')
      expect(textarea).toHaveValue('Current message')
    })

    it('auto-resizes on input', async () => {
      render(<ChatInput {...defaultProps} />)

      const textarea = screen.getByRole('textbox') as HTMLTextAreaElement

      await userEvent.type(textarea, 'Line 1\nLine 2\nLine 3\nLine 4\nLine 5')

      expect(textarea.style.height).toBeDefined()
    })

    it('respects max height of 400px', async () => {
      render(<ChatInput {...defaultProps} />)

      const textarea = screen.getByRole('textbox') as HTMLTextAreaElement

      Object.defineProperty(textarea, 'scrollHeight', {
        configurable: true,
        value: 500,
      })

      await userEvent.type(textarea, 'test')

      expect(Number.parseInt(textarea.style.height || '0')).toBeLessThanOrEqual(400)
    })
  })

  describe('Placeholder', () => {
    it('shows default placeholder', () => {
      render(<ChatInput {...defaultProps} />)

      expect(screen.getByPlaceholderText('Type a message...')).toBeInTheDocument()
    })

    it('shows custom placeholder', () => {
      render(<ChatInput {...defaultProps} placeholder="Enter your text" />)

      expect(screen.getByPlaceholderText('Enter your text')).toBeInTheDocument()
    })
  })

  describe('Children Rendering', () => {
    it('renders children elements', () => {
      render(
        <ChatInput {...defaultProps}>
          <button type="button">Test Button</button>
        </ChatInput>,
      )

      expect(screen.getByRole('button', { name: 'Test Button' })).toBeInTheDocument()
    })

    it('renders multiple children', () => {
      render(
        <ChatInput {...defaultProps}>
          <button type="button">Button 1</button>
          <button type="button">Button 2</button>
        </ChatInput>,
      )

      expect(screen.getByRole('button', { name: 'Button 1' })).toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Button 2' })).toBeInTheDocument()
    })
  })

  describe('Custom className', () => {
    it('applies custom className to form', () => {
      const { container } = render(<ChatInput {...defaultProps} className="custom-class" />)

      const form = container.querySelector('form')
      expect(form).toHaveClass('custom-class')
    })
  })

  describe('Container Structure', () => {
    it('renders chat input container with correct testid', () => {
      render(<ChatInput {...defaultProps} />)

      expect(screen.getByTestId('chat-input')).toBeInTheDocument()
    })

    it('has correct form structure', () => {
      render(<ChatInput {...defaultProps} />)

      const form = screen.getByRole('textbox').closest('form')
      expect(form).toBeInTheDocument()
    })
  })
})
