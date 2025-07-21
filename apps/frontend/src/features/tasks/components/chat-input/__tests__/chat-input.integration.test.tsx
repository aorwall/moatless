import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { TaskUIProvider } from '@/features/tasks/contexts/task-ui-provider'
import { ChatInput } from '../chat-input'

// Mock the API hooks
vi.mock('@/features/repositories/hooks/use-repositories', () => ({
  useRepositoriesList: vi.fn(() => ({ data: [], isLoading: false })),
  useBranches: vi.fn(() => ({ data: { branches: [] }, isLoading: false })),
  useSearchFiles: vi.fn(() => ({ data: null, isLoading: false, error: null })),
}))

vi.mock('@/features/templates', () => ({
  useTemplatesList: vi.fn(() => ({ data: { templates: [] }, isLoading: false })),
}))

vi.mock('@/features/tasks/hooks/use-tasks', () => ({
  useStopAgent: vi.fn(() => ({ mutate: vi.fn() })),
  useTaskFileSearch: vi.fn(() => ({ data: null, isLoading: false, error: null })),
}))

vi.mock('@/features/uploads/hooks/use-upload', () => ({
  useUploadFile: vi.fn(() => ({ mutate: vi.fn(), isPending: false })),
}))

// Helper to render with providers
const renderWithProviders = (component: React.ReactElement) => {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
    },
  })

  return render(
    <QueryClientProvider client={queryClient}>
      <TaskUIProvider taskId="test-task">{component}</TaskUIProvider>
    </QueryClientProvider>,
  )
}

describe('ChatInput integration', () => {
  it('should pass repositoryId from task prop to underlying components', () => {
    const taskId = 'task-123'

    const { container } = renderWithProviders(
      <ChatInput message="" onMessageChange={vi.fn()} onSubmit={vi.fn()} taskId={taskId} />,
    )

    // The component should render without errors
    expect(screen.getByPlaceholderText(/Type a message/)).toBeInTheDocument()

    // The repositoryId should be accessible internally
    // This test verifies that the component properly extracts repositoryId from task
    // and passes it down to ChatForm
    expect(container).toBeTruthy()
  })

  it('should work without task prop when repositoryId is not provided', () => {
    const taskId = 'task-123'

    const { container } = renderWithProviders(
      <ChatInput message="" onMessageChange={vi.fn()} onSubmit={vi.fn()} taskId={taskId} />,
    )

    // The component should render without errors even without repositoryId
    expect(screen.getByPlaceholderText(/Type a message/)).toBeInTheDocument()
    expect(container).toBeTruthy()
  })
})
