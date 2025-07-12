import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { useCreateTask } from '@/features/tasks/hooks/use-tasks'
import {
  useBranchSelector,
  useModelSelector,
  useRepositorySelector,
} from '../../chat-input/selectors'
import { CreateCodeTask } from '../create-code-task'

// Mock the hooks
vi.mock('@/features/tasks/hooks/use-tasks')
vi.mock('../../chat-input/selectors')
vi.mock('sonner', () => ({
  toast: {
    error: vi.fn(),
  },
}))

// Mock file search hooks
vi.mock('@/features/tasks/hooks/use-file-search', () => ({
  useTaskFileSearch: vi.fn(() => ({
    data: null,
    isLoading: false,
    error: null,
  })),
}))

vi.mock('@/features/repositories/hooks/use-repositories', () => ({
  useRepositoryFileSearch: vi.fn(() => ({
    data: null,
    isLoading: false,
    error: null,
  })),
  useSearchFiles: vi.fn(() => ({
    data: null,
    isLoading: false,
    error: null,
  })),
  useRepositoriesList: vi.fn(() => ({
    data: [],
    isLoading: false,
  })),
  useBranches: vi.fn(() => ({
    data: null,
    isLoading: false,
  })),
}))

vi.mock('@/features/environments/hooks/use-environments', () => ({
  useEnvironmentFileSearch: vi.fn(() => ({
    data: null,
    isLoading: false,
    error: null,
  })),
}))

const mockCreateTask = {
  mutate: vi.fn(),
  isPending: false,
}

const mockRepoSelector = {
  value: 'repo-123',
  onChange: vi.fn(),
  loading: false,
  placeholder: 'Select repository',
  items: [
    { id: 'repo-123', label: 'Test Repo 1' },
    { id: 'repo-456', label: 'Test Repo 2' },
  ],
  width: 'w-[180px]',
}

const mockBranchSelector = {
  value: 'main',
  onChange: vi.fn(),
  loading: false,
  placeholder: 'Select branch',
  items: [
    { id: 'main', label: 'main' },
    { id: 'develop', label: 'develop' },
  ],
  width: 'w-[140px]',
  disabled: false,
}

const mockModelSelector = {
  value: 'claude-3-opus-20240229',
  onChange: vi.fn(),
  loading: false,
  placeholder: 'Select model',
  items: [{ id: 'claude-3-opus-20240229', label: 'Claude 3 Opus' }],
  width: 'w-[180px]',
}

function renderComponent(props = {}) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  })

  return render(
    <QueryClientProvider client={queryClient}>
      <CreateCodeTask {...props} />
    </QueryClientProvider>,
  )
}

describe('CreateCodeTask', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(useCreateTask).mockReturnValue(mockCreateTask)
    vi.mocked(useRepositorySelector).mockReturnValue(mockRepoSelector)
    vi.mocked(useBranchSelector).mockReturnValue(mockBranchSelector)
    vi.mocked(useModelSelector).mockReturnValue(mockModelSelector)
  })

  it('renders with title when showTitle is true', () => {
    renderComponent({ showTitle: true, title: 'Custom Title' })

    expect(screen.getByText('Custom Title')).toBeInTheDocument()
  })

  it('does not render title when showTitle is false', () => {
    renderComponent({ showTitle: false, title: 'Custom Title' })

    expect(screen.queryByText('Custom Title')).not.toBeInTheDocument()
  })

  it('shows all selectors', () => {
    renderComponent()

    // Check for selector trigger buttons
    expect(screen.getByText('Test Repo 1')).toBeInTheDocument() // Selected repo
    expect(screen.getByText('main')).toBeInTheDocument() // Selected branch
    expect(screen.getByText('Claude 3 Opus')).toBeInTheDocument() // Selected model
  })

  it('disables submit button when message is empty', () => {
    renderComponent()

    const submitButton = screen.getByRole('button', { name: /start task/i })
    expect(submitButton).toBeDisabled()
  })

  it('enables submit button when message is entered and repository is selected', async () => {
    const user = userEvent.setup()
    renderComponent()

    const textarea = screen.getByPlaceholderText(/describe your coding task/i)
    await user.type(textarea, 'Build a new feature')

    const submitButton = screen.getByRole('button', { name: /start task/i })
    expect(submitButton).toBeEnabled()
  })

  it('calls createTask mutation with correct parameters on submit', async () => {
    const user = userEvent.setup()
    renderComponent()

    const textarea = screen.getByPlaceholderText(/describe your coding task/i)
    await user.type(textarea, 'Build a new feature')

    const submitButton = screen.getByRole('button', { name: /start task/i })
    await user.click(submitButton)

    expect(mockCreateTask.mutate).toHaveBeenCalledWith(
      {
        agentType: 'claude-code',
        message: 'Build a new feature',
        repositoryId: 'repo-123',
        model: 'claude-3-opus-20240229',
        filePaths: [],
        originalBranch: 'main',
      },
      expect.any(Object),
    )
  })

  it('clears message after successful submission', async () => {
    const user = userEvent.setup()
    renderComponent()

    const textarea = screen.getByPlaceholderText(/describe your coding task/i)
    await user.type(textarea, 'Build a new feature')

    // Simulate successful submission
    mockCreateTask.mutate.mockImplementationOnce((data, callbacks) => {
      callbacks.onSuccess()
    })

    const submitButton = screen.getByRole('button', { name: /start task/i })
    await user.click(submitButton)

    await waitFor(() => {
      expect(textarea).toHaveValue('')
    })
  })

  it('shows loading state when creating task', () => {
    vi.mocked(useCreateTask).mockReturnValue({
      ...mockCreateTask,
      isPending: true,
    })

    renderComponent()

    const submitButton = screen.getByRole('button', { name: /starting/i })
    expect(submitButton).toBeDisabled()
  })

  it('saves selected repository to localStorage', async () => {
    const localStorageSpy = vi.spyOn(Storage.prototype, 'setItem')

    const mockRepoSelectorWithChange = {
      ...mockRepoSelector,
      value: 'repo-456',
    }
    vi.mocked(useRepositorySelector).mockReturnValue(mockRepoSelectorWithChange)

    renderComponent()

    expect(localStorageSpy).toHaveBeenCalledWith('lastSelectedRepositoryId', 'repo-456')
  })

  it('initializes with repository from props', () => {
    const mockRepoSelectorWithDefault = {
      ...mockRepoSelector,
      value: 'default-repo',
    }
    vi.mocked(useRepositorySelector).mockImplementation((enabled, initialValue) => {
      if (initialValue === 'default-repo') {
        return mockRepoSelectorWithDefault
      }
      return mockRepoSelector
    })

    renderComponent({ defaultRepositoryId: 'default-repo' })

    expect(useRepositorySelector).toHaveBeenCalledWith(true, 'default-repo')
  })

  it('initializes with default message', () => {
    renderComponent({ defaultMessage: 'Default task message' })

    const textarea = screen.getByPlaceholderText(/describe your coding task/i)
    expect(textarea).toHaveValue('Default task message')
  })
})
