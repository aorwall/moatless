import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { FileText, Zap } from 'lucide-react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useCreateVibeTask } from '@/features/templates'
import { useModelSelector, useTemplateSelector } from '@/lib/components/chat/chat-input/selectors'
import { createMockMutationResult, createPendingMutation } from '@/test-utils/mock-mutations'
import { CreateVibeTask } from '../create-vibe-task'

// Mock the hooks
vi.mock('@/features/templates')
vi.mock('../../chat-input/selectors', () => ({
  GenericSelector: ({ value, items }: any) => {
    const item = items?.find((i: any) => i.id === value)
    return item ? <span>{item.label}</span> : null
  },
  useTemplateSelector: vi.fn(),
  useModelSelector: vi.fn(),
}))
vi.mock('sonner', () => ({
  toast: {
    error: vi.fn(),
  },
}))

// Mock the useTaskFileSearch from the use-tasks module
vi.mock('@/features/tasks/hooks/use-tasks', () => ({
  ...vi.importActual('@/features/tasks/hooks/use-tasks'),
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
  icon: FileText,
  tooltip: 'Select template',
  displayValue: 'Basic Hello Website',
}

const mockModelSelector = {
  value: 'claude-opus-4-20250514' as const,
  onChange: vi.fn(),
  loading: false,
  placeholder: 'Select model',
  items: [{ id: 'claude-opus-4-20250514' as const, label: 'Claude 3 Opus' }],
  width: 'w-[180px]',
  icon: Zap,
  tooltip: 'Select model',
  displayValue: 'Claude 3 Opus',
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
    templateId: 'hello-world-ui',
    onTemplateChange: vi.fn(),
  }

  return render(
    <QueryClientProvider client={queryClient}>
      <CreateVibeTask {...defaultProps} {...props} />
    </QueryClientProvider>,
  )
}

describe('CreateVibeTask', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(useCreateVibeTask).mockReturnValue(mockCreateVibeTask as any)
    vi.mocked(useTemplateSelector).mockReturnValue(mockTemplateSelector)
    vi.mocked(useModelSelector).mockReturnValue(mockModelSelector as any)
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
        message: 'Create a landing page',
        model: 'claude-opus-4-20250514',
        filePaths: [],
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
          message: 'Create a landing page',
          model: 'claude-opus-4-20250514',
          filePaths: [],
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

  it('does not include repository name in API call', async () => {
    const user = userEvent.setup()

    renderComponent()

    const textarea = screen.getByPlaceholderText(/describe what you want to build/i)
    await user.type(textarea, 'Create something')

    const submitButton = screen.getByRole('button', { name: /start task/i })
    await user.click(submitButton)

    expect(mockCreateVibeTask.mutate).toHaveBeenCalledWith(
      expect.objectContaining({
        templateId: 'hello-world-ui',
        message: 'Create something',
        filePaths: [],
      }),
      expect.any(Object),
    )

    // Verify repositoryName is NOT included
    const callArgs = vi.mocked(mockCreateVibeTask.mutate).mock.calls[0][0]
    expect(callArgs).not.toHaveProperty('repositoryName')
  })
})
