import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import React from 'react'
import { describe, expect, it, vi } from 'vitest'
import { ChatInput } from '../chat-input'
import { GenericSelector } from '../selectors'

// Mock the file search hooks
vi.mock('@/features/repositories/hooks/use-repositories', () => ({
  useSearchFiles: vi.fn(() => ({
    data: null,
    isLoading: false,
    error: null,
  })),
}))

vi.mock('@/features/tasks/hooks/use-tasks', () => ({
  useTaskFileSearch: vi.fn(() => ({
    data: null,
    isLoading: false,
    error: null,
  })),
}))

vi.mock('@/features/uploads/hooks/use-upload', () => ({
  useUploadFile: vi.fn(() => ({
    mutate: vi.fn(),
    isPending: false,
  })),
}))

// Helper to render with providers
const renderWithProviders = (component: React.ReactElement) => {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
    },
  })

  return render(<QueryClientProvider client={queryClient}>{component}</QueryClientProvider>)
}

describe('Refactored ChatInput (ChatForm replacement)', () => {
  it('renders with selectors', () => {
    const onSubmit = vi.fn()
    const onMessageChange = vi.fn()

    const modelItems = [
      { id: 'model1', label: 'Model 1' },
      { id: 'model2', label: 'Model 2' },
    ]

    const repoItems = [
      { id: 'repo1', label: 'Repo 1' },
      { id: 'repo2', label: 'Repo 2' },
    ]

    renderWithProviders(
      <ChatInput
        message=""
        onMessageChange={onMessageChange}
        onSubmit={onSubmit}
        placeholder="Type a message..."
        buttonText="Send"
        selectors={
          <>
            <GenericSelector
              key="repo"
              value=""
              onChange={() => {}}
              placeholder="Select repository"
              items={repoItems}
            />
            <GenericSelector
              key="model"
              value="model1"
              onChange={() => {}}
              placeholder="Select model"
              items={modelItems}
            />
          </>
        }
      />,
    )

    // Check that selectors are rendered
    expect(screen.getByText('Select repository')).toBeInTheDocument()
    // Use getAllByText since select components render the value multiple times
    expect(screen.getAllByText('Model 1')).toHaveLength(2)

    // Check that the textarea is rendered
    expect(screen.getByPlaceholderText('Type a message...')).toBeInTheDocument()

    // Check that the send button is rendered
    expect(screen.getByRole('button', { name: 'Send' })).toBeInTheDocument()
  })

  it('handles message input and submission', async () => {
    const user = userEvent.setup()
    const onSubmit = vi.fn()
    const onMessageChange = vi.fn()

    // Create a controlled component wrapper
    function TestWrapper() {
      const [message, setMessage] = React.useState('')

      return (
        <ChatInput
          message={message}
          onMessageChange={(newMessage) => {
            setMessage(newMessage)
            onMessageChange(newMessage)
          }}
          onSubmit={onSubmit}
          placeholder="Type a message..."
          buttonText="Send"
          selectors={<></>}
        />
      )
    }

    renderWithProviders(<TestWrapper />)

    const textarea = screen.getByPlaceholderText('Type a message...')

    // Type a message
    await user.type(textarea, 'Hello world')

    // Check that onChange was called
    expect(onMessageChange).toHaveBeenCalled()

    // Submit with Enter key
    await user.keyboard('{Enter}')

    // Check that onSubmit was called
    expect(onSubmit).toHaveBeenCalled()
  })

  it('accepts empty selectors array', () => {
    const onSubmit = vi.fn()
    const onMessageChange = vi.fn()

    render(
      <ChatInput message="" onMessageChange={onMessageChange} onSubmit={onSubmit} selectors={<></>} />,
    )

    // Should render without selectors
    expect(screen.getByRole('textbox')).toBeInTheDocument()
  })
})
