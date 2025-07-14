import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useSearchFiles } from '@/features/environments/hooks/use-environments'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { createWrapper } from '@/test-utils/render'
import { ChatInput } from '../chat-input'

// Mock dependencies
vi.mock('@/features/repositories/hooks/use-repositories', () => ({
  useRepositoriesList: vi.fn(),
}))

vi.mock('@/features/environments/hooks/use-environments', () => ({
  useSearchFiles: vi.fn(),
}))

// Mock child components to avoid complex dependencies
vi.mock('../chat-form', () => ({
  ChatForm: ({ onSubmit, placeholder, isSubmitting }: any) => (
    <form
      onSubmit={(e) => {
        e.preventDefault()
        const formData = new FormData(e.currentTarget)
        onSubmit(formData.get('message'), vi.fn(), true)
      }}
    >
      <textarea
        name="message"
        placeholder={placeholder}
        disabled={isSubmitting}
        data-testid="message-input"
      />
      <button type="submit" disabled={isSubmitting}>
        Submit
      </button>
    </form>
  ),
}))

// Mock repository selector
vi.mock('../selectors', () => ({
  RepositorySelector: ({ value, onChange }: any) => (
    <select
      value={value}
      onChange={(e) => onChange(e.target.value)}
      data-testid="repository-selector"
    >
      <option value="">Select repository</option>
      <option value="repo-123">Test Repository</option>
    </select>
  ),
}))

describe('ChatInput File Search', () => {
  const mockRepositoriesList = vi.mocked(useRepositoriesList)
  const mockSearchFiles = vi.mocked(useSearchFiles)

  const mockFileSearchResponse = {
    files: [
      { path: 'src/components/Button.tsx', relativePath: 'src/components/Button.tsx' },
      { path: 'src/utils/helpers.ts', relativePath: 'src/utils/helpers.ts' },
    ],
  }

  beforeEach(() => {
    vi.clearAllMocks()

    // Default mock implementations
    mockRepositoriesList.mockReturnValue({
      data: [],
      isLoading: false,
      error: null,
    } as any)

    mockSearchFiles.mockReturnValue({
      data: null,
      isLoading: false,
      error: null,
    } as any)

    mockTaskFileSearch.mockReturnValue({
      data: null,
      isLoading: false,
      error: null,
    } as any)
  })

  it('renders with correct placeholder for new task', () => {
    render(<ChatInput onSendMessage={vi.fn()} />, { wrapper: createWrapper() })

    expect(screen.getByPlaceholderText(/Describe your coding task/)).toBeInTheDocument()
  })

  it('renders with correct placeholder when task is provided', () => {
    const mockTask = {
      iterations: [],
    }

    render(
      <ChatInput
        taskId="task-123"
        task={mockTask as any}
        taskStatus="open"
        onSendMessage={vi.fn()}
      />,
      { wrapper: createWrapper() },
    )

    expect(screen.getByPlaceholderText(/Type a new request/)).toBeInTheDocument()
  })

  it('disables input when task is closed', () => {
    const mockTask = {
      iterations: [],
    }

    render(
      <ChatInput
        taskId="task-123"
        task={mockTask as any}
        taskStatus="closed"
        onSendMessage={vi.fn()}
      />,
      { wrapper: createWrapper() },
    )

    const textarea = screen.getByTestId('message-input')
    expect(textarea).toBeDisabled()
  })

  it('handles message submission correctly', async () => {
    const user = userEvent.setup()
    const onSendMessage = vi.fn()

    render(<ChatInput onSendMessage={onSendMessage} />, { wrapper: createWrapper() })

    const textarea = screen.getByTestId('message-input')
    const submitButton = screen.getByText('Submit')

    await user.type(textarea, 'Test message')
    await user.click(submitButton)

    expect(onSendMessage).toHaveBeenCalledWith(
      undefined, // taskId
      'Test message',
      expect.any(Function),
      true,
      undefined,
      undefined,
    )
  })
})
