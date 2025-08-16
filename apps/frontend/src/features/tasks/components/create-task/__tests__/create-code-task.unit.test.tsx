import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { Folder, GitBranch, Settings, Zap } from 'lucide-react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useCreateTask } from '@/features/tasks/hooks/use-tasks'
import { createMockMutationResult, createPendingMutation } from '@/test-utils/mock-mutations'
import {
  useAgentModeSelector,
  useBranchSelector,
  useModelSelector,
  useRepositorySelector,
} from '../../chat-input/selectors'
import { CreateCodeTask } from '../create-code-task'

vi.mock('../../chat-input/selectors', () => ({
  GenericSelector: ({ value, items }: any) => {
    const item = items?.find((i: any) => i.id === value)
    return item ? <span>{item.label}</span> : null
  },
  useAgentModeSelector: vi.fn(),
  useBranchSelector: vi.fn(),
  useModelSelector: vi.fn(),
  useRepositorySelector: vi.fn(),
}))
vi.mock('sonner', () => ({
  toast: {
    error: vi.fn(),
  },
}))

// Mock the useTaskFileSearch from the use-tasks module
vi.mock('@/features/tasks/hooks/use-tasks', () => ({
  ...vi.importActual('@/features/tasks/hooks/use-tasks'),
  useCreateTask: vi.fn(),
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

const mockCreateTask = createMockMutationResult({
  mutate: vi.fn(),
  isPending: false,
})

const mockRepoSelector = {
  value: 'repo-123',
  onChange: vi.fn(),
  loading: false,
  error: null,
  isError: false,
  placeholder: 'Select repository',
  items: [
    { id: 'repo-123', label: 'Test Repo 1' },
    { id: 'repo-456', label: 'Test Repo 2' },
  ],
  width: 'w-[180px]',
  disabled: false,
  icon: Folder,
  tooltip: 'Select repository',
  displayValue: 'Test Repo 1',
}

const mockBranchSelector = {
  value: 'main',
  onChange: vi.fn(),
  loading: false,
  error: null,
  isError: false,
  placeholder: 'Select branch',
  items: [
    { id: 'main', label: 'main' },
    { id: 'develop', label: 'develop' },
  ],
  width: 'w-[140px]',
  disabled: false,
  icon: GitBranch,
  tooltip: 'Select branch',
  displayValue: 'main',
  currentBranch: 'main',
}

const mockModelSelector = {
  value: 'claude-opus-4-20250514' as const,
  onChange: vi.fn(),
  loading: false,
  error: null,
  isError: false,
  placeholder: 'Select model',
  items: [{ id: 'claude-opus-4-20250514' as const, label: 'Claude 3 Opus' }],
  width: 'w-[180px]',
  disabled: false,
  icon: Zap,
  tooltip: 'Select model',
  displayValue: 'Claude 3 Opus',
}

const mockAgentModeSelector = {
  value: 'edit' as const,
  onChange: vi.fn(),
  loading: false,
  error: null,
  isError: false,
  placeholder: 'Select mode',
  items: [
    { id: 'edit' as const, label: 'Edit' },
    { id: 'plan' as const, label: 'Plan' },
  ],
  width: 'w-[120px]',
  disabled: false,
  icon: Settings,
  tooltip: 'Select agent mode',
  displayValue: 'Edit',
}

function renderComponent(props = {}) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  })

  const defaultProps = {
    model: 'claude-opus-4-20250514' as const,
    onModelChange: vi.fn(),
    agentMode: 'edit' as const,
    onAgentModeChange: vi.fn(),
    repositoryId: 'repo-123',
    onRepositoryChange: vi.fn(),
  }

  return render(
    <QueryClientProvider client={queryClient}>
      <CreateCodeTask {...defaultProps} {...props} />
    </QueryClientProvider>,
  )
}

describe('CreateCodeTask', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(useCreateTask).mockReturnValue(mockCreateTask as any)
    vi.mocked(useRepositorySelector).mockReturnValue(mockRepoSelector)
    vi.mocked(useBranchSelector).mockReturnValue(mockBranchSelector)
    vi.mocked(useModelSelector).mockReturnValue(mockModelSelector as any)
    vi.mocked(useAgentModeSelector).mockReturnValue(mockAgentModeSelector as any)
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
        model: 'claude-opus-4-20250514',
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
    vi.mocked(mockCreateTask.mutate).mockImplementationOnce((_data: any, callbacks: any) => {
      callbacks?.onSuccess?.(
        {
          status: 'success',
          taskId: 'test-task-id',
        },
        {
          agentType: 'claude-code',
          message: 'Build a new feature',
          repositoryId: 'repo-123',
          model: 'claude-opus-4-20250514',
          filePaths: [],
          originalBranch: 'main',
        },
        undefined,
      )
    })

    const submitButton = screen.getByRole('button', { name: /start task/i })
    await user.click(submitButton)

    await waitFor(() => {
      expect(textarea).toHaveValue('')
    })
  })

  it('shows loading state when creating task', () => {
    vi.mocked(useCreateTask).mockReturnValue(createPendingMutation() as any)

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
    vi.mocked(useRepositorySelector).mockImplementation((_enabled: any, initialValue: any) => {
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
