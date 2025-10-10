import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen } from '@testing-library/react'
import { userEvent } from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { CreateCodeTask } from '../create-code-task'

// Mock dependencies
vi.mock('@tanstack/react-router', () => ({
  useNavigate: vi.fn(() => vi.fn()),
}))

vi.mock('@/features/tasks/hooks/use-tasks', () => ({
  useCreateTask: vi.fn(() => ({
    mutate: vi.fn(),
    isPending: false,
  })),
}))

vi.mock('@/features/uploads/hooks/use-upload', () => ({
  useUploadFile: vi.fn(() => ({
    mutate: vi.fn(),
    isPending: false,
  })),
}))

vi.mock('@/features/repositories/hooks/use-repositories', () => ({
  useRepositoriesList: vi.fn(() => ({
    data: [{ id: 'repo-1', name: 'Test Repo' }],
    isLoading: false,
  })),
  useBranches: vi.fn(() => ({
    data: { branches: ['main'], currentBranch: 'main' },
    isLoading: false,
  })),
}))

describe('CreateCodeTask', () => {
  let queryClient: QueryClient

  beforeEach(() => {
    queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    })
    vi.clearAllMocks()
  })

  it('should allow submitting with empty message when repository is selected', async () => {
    const user = userEvent.setup()
    const mockMutate = vi.fn()

    const { useCreateTask } = await import('@/features/tasks/hooks/use-tasks')
    vi.mocked(useCreateTask).mockReturnValue({
      mutate: mockMutate,
      isPending: false,
    } as any)

    render(
      <QueryClientProvider client={queryClient}>
        <CreateCodeTask
          model="claude-opus-4-20250514"
          onModelChange={vi.fn()}
          agentMode="edit"
          onAgentModeChange={vi.fn()}
          repositoryId="repo-1"
          onRepositoryChange={vi.fn()}
        />
      </QueryClientProvider>,
    )

    const sendButton = screen.getByRole('button', { name: /Create Task/i })

    expect(sendButton).not.toBeDisabled()

    await user.click(sendButton)

    expect(mockMutate).toHaveBeenCalledWith(
      expect.objectContaining({
        repositoryId: 'repo-1',
        model: 'claude-opus-4-20250514',
        agentMode: 'edit',
        message: undefined,
      }),
      expect.any(Object),
    )
  })

  it('should enable and submit when message is provided', async () => {
    const user = userEvent.setup()
    const mockMutate = vi.fn()

    const { useCreateTask } = await import('@/features/tasks/hooks/use-tasks')
    vi.mocked(useCreateTask).mockReturnValue({
      mutate: mockMutate,
      isPending: false,
    } as any)

    render(
      <QueryClientProvider client={queryClient}>
        <CreateCodeTask
          model="claude-opus-4-20250514"
          onModelChange={vi.fn()}
          agentMode="edit"
          onAgentModeChange={vi.fn()}
          repositoryId="repo-1"
          onRepositoryChange={vi.fn()}
          defaultMessage="Build a feature"
        />
      </QueryClientProvider>,
    )

    const sendButton = screen.getByRole('button', { name: /Start Task/i })

    expect(sendButton).not.toBeDisabled()

    await user.click(sendButton)

    expect(mockMutate).toHaveBeenCalled()
  })

  it('should disable button when no repository is selected', () => {
    render(
      <QueryClientProvider client={queryClient}>
        <CreateCodeTask
          model="claude-opus-4-20250514"
          onModelChange={vi.fn()}
          agentMode="edit"
          onAgentModeChange={vi.fn()}
          repositoryId=""
          onRepositoryChange={vi.fn()}
          defaultMessage="Build a feature"
        />
      </QueryClientProvider>,
    )

    const sendButton = screen.getByRole('button', { name: /Start Task/i })

    expect(sendButton).toBeDisabled()
  })
})
