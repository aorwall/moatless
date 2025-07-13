import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useCreateVibeTask } from '@/features/templates'
import { createMockMutationResult, createPendingMutation } from '@/test-utils/mock-mutations'
import { useModelSelector, useTemplateSelector } from '../../chat-input/selectors'
import { CreateVibeTask } from '../create-vibe-task'

// Mock the hooks
vi.mock('@/features/templates')
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
}))

vi.mock('@/features/environments/hooks/use-environments', () => ({
  useEnvironmentFileSearch: vi.fn(() => ({
    data: null,
    isLoading: false,
    error: null,
  })),
}))

const mockCreateVibeTask = createMockMutationResult({
  mutate: vi.fn(),
  isPending: false,
})

const mockTemplateSelector = {
  value: 'hello-world-ui',
  onChange: vi.fn(),
  loading: false,
  placeholder: 'Select template',
  items: [
    { id: 'hello-world-ui', label: 'Basic Hello Website' },
    { id: 'react-app', label: 'React Application' },
  ],
  width: 'w-[180px]',
}

const mockModelSelector = {
  value: 'claude-opus-4-20250514' as const,
  onChange: vi.fn(),
  loading: false,
  placeholder: 'Select model',
  items: [{ id: 'claude-opus-4-20250514' as const, label: 'Claude Opus 4' }],
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
      <CreateVibeTask {...props} />
    </QueryClientProvider>,
  )
}

describe('CreateVibeTask', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(useCreateVibeTask).mockReturnValue(mockCreateVibeTask as any)
    vi.mocked(useTemplateSelector).mockReturnValue(mockTemplateSelector)
    vi.mocked(useModelSelector).mockReturnValue(mockModelSelector as any)

    // Mock Date.now() for consistent repository naming
    vi.spyOn(Date, 'now').mockReturnValue(1234567890)
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('renders with title when showTitle is true', () => {
    renderComponent({ showTitle: true, title: 'Vibe Title' })

    expect(screen.getByText('Vibe Title')).toBeInTheDocument()
  })

  it('does not render title when showTitle is false', () => {
    renderComponent({ showTitle: false, title: 'Vibe Title' })

    expect(screen.queryByText('Vibe Title')).not.toBeInTheDocument()
  })

  it('shows template and model selectors', () => {
    renderComponent()

    // Check for selector values
    expect(screen.getByText('Basic Hello Website')).toBeInTheDocument() // Selected template
    expect(screen.getByText('Claude 3 Opus')).toBeInTheDocument() // Selected model
  })

  it('disables submit button when message is empty', () => {
    renderComponent()

    const submitButton = screen.getByRole('button', { name: /start task/i })
    expect(submitButton).toBeDisabled()
  })

  it('enables submit button when message is entered and template is selected', async () => {
    const user = userEvent.setup()
    renderComponent()

    const textarea = screen.getByPlaceholderText(/describe what you want to build/i)
    await user.type(textarea, 'Create a landing page')

    const submitButton = screen.getByRole('button', { name: /start task/i })
    expect(submitButton).toBeEnabled()
  })

  it('calls createVibeTask mutation with correct parameters on submit', async () => {
    const user = userEvent.setup()
    renderComponent()

    const textarea = screen.getByPlaceholderText(/describe what you want to build/i)
    await user.type(textarea, 'Create a landing page')

    const submitButton = screen.getByRole('button', { name: /start task/i })
    await user.click(submitButton)

    expect(mockCreateVibeTask.mutate).toHaveBeenCalledWith(
      {
        agentType: 'claude-code',
        templateId: 'hello-world-ui',
        repositoryName: 'hello-world-ui-1234567890',
        message: 'Create a landing page',
        model: 'claude-3-opus-20240229',
        filePaths: [],
        start: true,
      },
      expect.any(Object),
    )
  })

  it('clears message after successful submission', async () => {
    const user = userEvent.setup()
    renderComponent()

    const textarea = screen.getByPlaceholderText(/describe what you want to build/i)
    await user.type(textarea, 'Create a landing page')

    // Simulate successful submission
    vi.mocked(mockCreateVibeTask.mutate).mockImplementationOnce((_data: any, callbacks: any) => {
      callbacks?.onSuccess?.(
        {
          taskId: 'test-task-id',
          status: 'success',
        },
        {
          agentType: 'vibe',
          templateId: 'hello-world-ui',
          repositoryName: 'hello-world-ui-1234567890',
          message: 'Create a landing page',
          model: 'claude-opus-4-20250514',
          filePaths: [],
          start: true,
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
    vi.mocked(useCreateVibeTask).mockReturnValue(createPendingMutation() as any)

    renderComponent()

    const submitButton = screen.getByRole('button', { name: /starting/i })
    expect(submitButton).toBeDisabled()
  })

  it('saves selected template to localStorage', async () => {
    const localStorageSpy = vi.spyOn(Storage.prototype, 'setItem')

    const mockTemplateSelectorWithChange = {
      ...mockTemplateSelector,
      value: 'react-app',
    }
    vi.mocked(useTemplateSelector).mockReturnValue(mockTemplateSelectorWithChange)

    renderComponent()

    expect(localStorageSpy).toHaveBeenCalledWith('lastSelectedTemplateId', 'react-app')
  })

  it('initializes template from localStorage', () => {
    const localStorageGetSpy = vi
      .spyOn(Storage.prototype, 'getItem')
      .mockReturnValue('saved-template')
    const mockTemplateSelectorEmpty = {
      ...mockTemplateSelector,
      value: '',
    }
    vi.mocked(useTemplateSelector).mockReturnValue(mockTemplateSelectorEmpty)

    renderComponent()

    expect(localStorageGetSpy).toHaveBeenCalledWith('lastSelectedTemplateId')
    expect(mockTemplateSelectorEmpty.onChange).toHaveBeenCalledWith('saved-template')
  })

  it('auto-selects first template when none is selected', () => {
    const mockTemplateSelectorEmpty = {
      ...mockTemplateSelector,
      value: '',
    }
    vi.mocked(useTemplateSelector).mockReturnValue(mockTemplateSelectorEmpty)

    renderComponent()

    expect(mockTemplateSelectorEmpty.onChange).toHaveBeenCalledWith('hello-world-ui')
  })

  it('initializes with default message', () => {
    renderComponent({ defaultMessage: 'Default vibe message' })

    const textarea = screen.getByPlaceholderText(/describe what you want to build/i)
    expect(textarea).toHaveValue('Default vibe message')
  })

  it('generates unique repository name based on template and timestamp', async () => {
    const user = userEvent.setup()

    // Test with different timestamps
    vi.spyOn(Date, 'now').mockReturnValue(9876543210)

    renderComponent()

    const textarea = screen.getByPlaceholderText(/describe what you want to build/i)
    await user.type(textarea, 'Create something')

    const submitButton = screen.getByRole('button', { name: /start task/i })
    await user.click(submitButton)

    expect(mockCreateVibeTask.mutate).toHaveBeenCalledWith(
      expect.objectContaining({
        repositoryName: 'hello-world-ui-9876543210',
      }),
      expect.any(Object),
    )
  })
})
