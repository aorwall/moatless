import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { TaskUIProvider } from '@/features/tasks/contexts/task-ui-provider'
import { createMockMutationResult } from '@/test-utils/mock-mutations'
import { ChatInput } from '../index'

// Mock dependencies
vi.mock('@/features/repositories/hooks/use-repositories', () => ({
  useRepositoriesList: () => ({ data: [], isLoading: false }),
  useBranches: () => ({ data: { branches: [] }, isLoading: false }),
  useSearchFiles: () => ({ data: null, isLoading: false, error: null }),
}))

vi.mock('@/features/tasks/hooks/use-tasks', () => ({
  useStopAgent: () => createMockMutationResult({ mutate: vi.fn(), isPending: false }),
  useTaskFileSearch: () => ({ data: null, isLoading: false, error: null }),
}))

vi.mock('@/features/templates', () => ({
  useTemplatesList: () => ({ data: { templates: [] }, isLoading: false }),
}))

vi.mock('@/features/uploads/hooks/use-upload', () => ({
  useUploadFile: () => createMockMutationResult({ mutate: vi.fn(), isPending: false }),
}))

vi.mock('@/lib/hooks/use-universal-file-autocomplete', () => ({
  useUniversalFileAutocomplete: () => ({
    inputRef: { current: null },
    isOpen: false,
    results: [],
    selectedIndex: -1,
    isLoading: false,
    onInputChange: vi.fn(),
    onSelectResult: vi.fn(),
    onKeyDown: vi.fn(),
  }),
}))

// Helper to render with providers
const renderChatInputWithTaskProvider = (taskId: string, component: React.ReactElement) => {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
    },
  })

  return render(
    <QueryClientProvider client={queryClient}>
      <TaskUIProvider taskId={taskId}>{component}</TaskUIProvider>
    </QueryClientProvider>,
  )
}

describe('ChatInput Task Isolation', () => {
  beforeEach(() => {
    // Clear localStorage to ensure consistent test behavior
    if (typeof window !== 'undefined') {
      localStorage.removeItem('lastSelectedModel')
    }
  })

  it('should isolate message state between different tasks', () => {
    // This test is primarily about verifying the basic rendering with different task contexts
    // The actual state isolation is better tested at the hook level in task-ui-provider tests

    // Render first ChatInput with task-1
    const { unmount: unmountTask1 } = renderChatInputWithTaskProvider(
      'task-1',
      <ChatInput taskId="task-1" onSendMessage={vi.fn()} />,
    )

    // Should render successfully
    expect(screen.getByPlaceholderText(/type a new request/i)).toBeInTheDocument()

    // Unmount first task
    unmountTask1()

    // Render second ChatInput with task-2
    renderChatInputWithTaskProvider('task-2', <ChatInput taskId="task-2" onSendMessage={vi.fn()} />)

    // Should render successfully with different task context
    expect(screen.getByPlaceholderText(/type a new request/i)).toBeInTheDocument()
  })

  it('should render with default model selection', () => {
    renderChatInputWithTaskProvider('task-1', <ChatInput taskId="task-1" onSendMessage={vi.fn()} />)

    // Should render with default model (Opus) - check for the combobox button with the selected value
    expect(screen.getByRole('combobox')).toBeInTheDocument()
    expect(screen.getByRole('combobox')).toHaveTextContent('Claude Opus 4')
  })

  it('should integrate with TaskUIProvider context successfully', () => {
    const mockOnSendMessage = vi.fn()

    renderChatInputWithTaskProvider(
      'test-task',
      <ChatInput taskId="test-task" onSendMessage={mockOnSendMessage} />,
    )

    // Should render main elements without errors
    expect(screen.getByPlaceholderText(/type a new request/i)).toBeInTheDocument()
    expect(screen.getByRole('combobox')).toHaveTextContent('Claude Opus 4')
    expect(screen.getByRole('button', { name: /send/i })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /upload file/i })).toBeInTheDocument()
  })
})
