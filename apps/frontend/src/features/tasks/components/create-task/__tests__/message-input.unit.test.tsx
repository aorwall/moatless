import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ChatInput } from '@/lib/components/chat/chat-input/chat-input'

// Mock the file autocomplete hook
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

// Mock the FileAutocomplete component
vi.mock('@/features/tasks/components/file-autocomplete', () => ({
  FileAutocomplete: ({ onSelect }: { onSelect: (file: any) => void }) => (
    <div data-testid="file-autocomplete">
      <button
        type="button"
        onClick={() => onSelect({ path: 'test.js', absolutePath: '/path/to/test.js', score: 1.0 })}
      >
        Select test.js
      </button>
    </div>
  ),
}))

// Mock upload hook
vi.mock('@/features/uploads/hooks/use-upload', () => ({
  useUploadFile: () => ({
    mutate: vi.fn(),
    isPending: false,
  }),
}))

describe('ChatInput (MessageInput replacement)', () => {
  it('renders with default props', () => {
    const handleChange = vi.fn()
    const handleSubmit = vi.fn()

    render(<ChatInput message="" onMessageChange={handleChange} onSubmit={handleSubmit} />)

    expect(screen.getByPlaceholderText('Type a message...')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Send' })).toBeInTheDocument()
  })

  it('calls onMessageChange when typing', () => {
    const handleChange = vi.fn()
    const handleSubmit = vi.fn()

    render(<ChatInput message="" onMessageChange={handleChange} onSubmit={handleSubmit} />)

    const textarea = screen.getByPlaceholderText('Type a message...')
    fireEvent.change(textarea, { target: { value: 'Hello world' } })

    expect(handleChange).toHaveBeenCalledWith('Hello world')
  })

  it('calls onSubmit when clicking send button', () => {
    const handleChange = vi.fn()
    const handleSubmit = vi.fn()

    render(
      <ChatInput message="Test message" onMessageChange={handleChange} onSubmit={handleSubmit} />,
    )

    const button = screen.getByRole('button')
    fireEvent.click(button)

    expect(handleSubmit).toHaveBeenCalled()
  })

  it('calls onSubmit when pressing Enter', () => {
    const handleChange = vi.fn()
    const handleSubmit = vi.fn()

    render(
      <ChatInput message="Test message" onMessageChange={handleChange} onSubmit={handleSubmit} />,
    )

    const textarea = screen.getByPlaceholderText('Type a message...')
    fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false })

    expect(handleSubmit).toHaveBeenCalled()
  })

  it('does not call onSubmit when pressing Shift+Enter', () => {
    const handleChange = vi.fn()
    const handleSubmit = vi.fn()

    render(
      <ChatInput message="Test message" onMessageChange={handleChange} onSubmit={handleSubmit} />,
    )

    const textarea = screen.getByPlaceholderText('Type a message...')
    fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: true })

    expect(handleSubmit).not.toHaveBeenCalled()
  })

  it('disables submit button when message is empty', () => {
    const handleChange = vi.fn()
    const handleSubmit = vi.fn()

    render(<ChatInput message="" onMessageChange={handleChange} onSubmit={handleSubmit} />)

    const button = screen.getByRole('button')
    expect(button).toBeDisabled()
  })

  it('disables submit button when disabled prop is true', () => {
    const handleChange = vi.fn()
    const handleSubmit = vi.fn()

    render(
      <ChatInput
        message="Test message"
        onMessageChange={handleChange}
        onSubmit={handleSubmit}
        disabled={true}
      />,
    )

    const button = screen.getByRole('button')
    expect(button).toBeDisabled()
  })

  it('shows loading state', () => {
    const handleChange = vi.fn()
    const handleSubmit = vi.fn()

    render(
      <ChatInput
        message="Test message"
        onMessageChange={handleChange}
        onSubmit={handleSubmit}
        loading={true}
      />,
    )

    const textarea = screen.getByPlaceholderText('Type a message...')
    expect(textarea).toBeDisabled()
  })

  it('renders custom button text in screen reader text', () => {
    const handleChange = vi.fn()
    const handleSubmit = vi.fn()

    render(
      <ChatInput
        message=""
        onMessageChange={handleChange}
        onSubmit={handleSubmit}
        buttonText="Submit"
      />,
    )

    expect(screen.getByText('Submit')).toBeInTheDocument()
  })

  it('renders children when provided', () => {
    const handleChange = vi.fn()
    const handleSubmit = vi.fn()

    render(
      <ChatInput message="" onMessageChange={handleChange} onSubmit={handleSubmit}>
        <div data-testid="test-selector">Test Selector</div>
      </ChatInput>,
    )

    expect(screen.getByTestId('test-selector')).toBeInTheDocument()
  })

  it('applies custom placeholder', () => {
    const handleChange = vi.fn()
    const handleSubmit = vi.fn()

    render(
      <ChatInput
        message=""
        onMessageChange={handleChange}
        onSubmit={handleSubmit}
        placeholder="Custom placeholder"
      />,
    )

    expect(screen.getByPlaceholderText('Custom placeholder')).toBeInTheDocument()
  })

  it('auto-resizes textarea when typing', () => {
    const handleChange = vi.fn()
    const handleSubmit = vi.fn()

    render(
      <ChatInput
        message=""
        onMessageChange={handleChange}
        onSubmit={handleSubmit}
        className="max-h-[150px]"
      />,
    )

    const textarea = screen.getByPlaceholderText('Type a message...')

    // Initially should have minimum height
    expect(textarea.style.minHeight).toBe('40px')

    // Mock scrollHeight property
    Object.defineProperty(textarea, 'scrollHeight', {
      configurable: true,
      value: 80,
    })

    // Type some text that would cause expansion
    fireEvent.change(textarea, {
      target: {
        value: 'This is a long message that should cause the textarea to expand in height',
      },
    })

    expect(handleChange).toHaveBeenCalledWith(
      'This is a long message that should cause the textarea to expand in height',
    )
    // The textarea should be resized to match scrollHeight
    expect(textarea.style.height).toBe('80px')
  })

  it('respects className for styling', () => {
    const handleChange = vi.fn()
    const handleSubmit = vi.fn()

    render(
      <ChatInput
        message=""
        onMessageChange={handleChange}
        onSubmit={handleSubmit}
        className="max-h-[100px]"
      />,
    )

    const textarea = screen.getByPlaceholderText('Type a message...')
    expect(textarea).toBeInTheDocument()
  })

  it('renders as a pure UI component without hooks', () => {
    const handleChange = vi.fn()
    const handleSubmit = vi.fn()

    render(<ChatInput message="@test" onMessageChange={handleChange} onSubmit={handleSubmit} />)

    // Should render successfully as a pure UI component
    expect(screen.getByPlaceholderText('Type a message...')).toBeInTheDocument()
  })
})
