import type { FileSearchResponse } from '@moatless/types/types/repository'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { useState } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  useBranches,
  useRepositoriesList,
  useSearchFiles,
} from '@/features/repositories/hooks/use-repositories'
import { TaskUIProvider } from '@/features/tasks/contexts/task-ui-provider'
import { useSendMessage, useStopAgent, useTaskFileSearch } from '@/features/tasks/hooks/use-tasks'
import { useTemplatesList } from '@/features/templates'
import { useUploadFile } from '@/features/uploads/hooks/use-upload'
import { ChatInput } from '../index'

// Mock the hooks
vi.mock('@/features/repositories/hooks/use-repositories')
vi.mock('@/features/tasks/hooks/use-tasks')
vi.mock('@/features/uploads/hooks/use-upload')
vi.mock('@/features/templates')

// Mock the universal file autocomplete hook
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

describe('ChatInput file search functionality', () => {
  const mockSearchFiles = vi.mocked(useSearchFiles)
  const mockTaskFileSearch = vi.mocked(useTaskFileSearch)
  const mockRepositoriesList = vi.mocked(useRepositoriesList)
  const mockBranches = vi.mocked(useBranches)
  const mockStopAgent = vi.mocked(useStopAgent)
  const mockSendMessage = vi.mocked(useSendMessage)
  const mockUploadFile = vi.mocked(useUploadFile)
  const mockTemplatesList = vi.mocked(useTemplatesList)

  const mockFileSearchResponse: FileSearchResponse = {
    results: [
      {
        path: 'src/components/Button.tsx',
        absolutePath: '/repo/src/components/Button.tsx',
        score: 0.9,
      },
      { path: 'src/utils/helpers.ts', absolutePath: '/repo/src/utils/helpers.ts', score: 0.8 },
    ],
    totalFound: 2,
    query: 'test',
  }

  const createWrapper = () => {
    const queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    })

    return ({ children }: { children: React.ReactNode }) => (
      <QueryClientProvider client={queryClient}>
        <TaskUIProvider taskId="test-task">{children}</TaskUIProvider>
      </QueryClientProvider>
    )
  }

  beforeEach(() => {
    vi.clearAllMocks()

    // Mock scrollIntoView for jsdom
    Element.prototype.scrollIntoView = vi.fn()

    // Default mock implementations
    mockSearchFiles.mockReturnValue({
      data: undefined,
      isLoading: false,
      error: null,
    } as any)

    mockTaskFileSearch.mockReturnValue({
      data: undefined,
      isLoading: false,
      error: null,
    } as any)

    mockRepositoriesList.mockReturnValue({
      data: [],
      isLoading: false,
      error: null,
    } as any)

    mockBranches.mockReturnValue({
      data: undefined,
      isLoading: false,
      error: null,
    } as any)

    mockStopAgent.mockReturnValue({
      mutate: vi.fn(),
      isPending: false,
    } as any)

    mockSendMessage.mockReturnValue({
      mutate: vi.fn(),
      isPending: false,
    } as any)

    mockUploadFile.mockReturnValue({
      mutate: vi.fn(),
      isPending: false,
    } as any)

    mockTemplatesList.mockReturnValue({
      data: { templates: [] },
      isLoading: false,
      error: null,
    } as any)
  })

  it.skip('should use repository search when in homepage context (no taskId) - FIXME: Autocomplete component testing issues (TASK-101)', async () => {
    const user = userEvent.setup()
    const repositoryId = 'repo-123'

    // Mock repositories data
    mockRepositoriesList.mockReturnValue({
      data: [{ id: repositoryId, name: 'test-repo' }],
      isLoading: false,
      error: null,
    } as any)

    // Mock successful search response
    mockSearchFiles.mockReturnValue({
      data: mockFileSearchResponse,
      isLoading: false,
      error: null,
    } as any)

    // Use a ChatInput with pre-selected repository to avoid testing the select component
    const TestWrapper = () => {
      const [_selectedRepo] = useState(repositoryId)
      return (
        <ChatInput
          taskId="test-task"
          onSendMessage={vi.fn()}
          // Pass repositoryId through a controlled component to simulate selection
        />
      )
    }

    render(<TestWrapper />, { wrapper: createWrapper() })

    // Wait for the component to be ready
    await waitFor(() => {
      expect(screen.getByPlaceholderText(/Describe your coding task/)).toBeInTheDocument()
    })

    const textarea = screen.getByPlaceholderText(/Describe your coding task/)

    // Type @ to trigger autocomplete
    await user.type(textarea, '@But')

    // Verify repository search was called with correct parameters
    await waitFor(() => {
      expect(mockSearchFiles).toHaveBeenCalledWith(
        repositoryId,
        'But',
        expect.objectContaining({
          enabled: true, // Should be enabled for homepage context
        }),
      )
    })

    // Task search should be called but disabled
    expect(mockTaskFileSearch).toHaveBeenCalledWith(
      '', // Empty taskId
      'But',
      repositoryId,
      expect.objectContaining({
        enabled: false, // Should be disabled for homepage context
      }),
    )
  })

  it.skip('should use task search when in task context with repositoryId - FIXME: Autocomplete component testing issues (TASK-101)', async () => {
    const user = userEvent.setup()
    const taskId = 'task-123'
    const repositoryId = 'repo-456'

    // Mock successful search response
    mockTaskFileSearch.mockReturnValue({
      data: mockFileSearchResponse,
      isLoading: false,
      error: null,
    } as any)

    // Mock task with repositoryId
    const mockTask = {
      iterations: [],
      repositoryId,
    }

    render(
      <ChatInput
        taskId={taskId}
        task={mockTask as any}
        taskStatus="open"
        onSendMessage={vi.fn()}
      />,
      { wrapper: createWrapper() },
    )

    const textarea = screen.getByPlaceholderText(/Type a new request/)

    // Type @ to trigger autocomplete
    await user.type(textarea, '@But')

    // Verify task search was called with correct parameters
    await waitFor(() => {
      expect(mockTaskFileSearch).toHaveBeenCalledWith(
        taskId,
        'But',
        repositoryId,
        expect.objectContaining({
          enabled: true,
        }),
      )
    })

    // Repository search should be called but disabled
    expect(mockSearchFiles).toHaveBeenCalledWith(
      repositoryId,
      'But',
      expect.objectContaining({
        enabled: false, // Disabled in task context
      }),
    )
  })

  it.skip('should show autocomplete results when search returns data - FIXME: Autocomplete component testing issues (TASK-101)', async () => {
    const user = userEvent.setup()
    const taskId = 'task-123'
    const repositoryId = 'repo-456'

    // Mock successful search response
    mockTaskFileSearch.mockReturnValue({
      data: mockFileSearchResponse,
      isLoading: false,
      error: null,
    } as any)

    const mockTask = {
      iterations: [],
      repositoryId,
    }

    render(
      <ChatInput
        taskId={taskId}
        task={mockTask as any}
        taskStatus="open"
        onSendMessage={vi.fn()}
      />,
      { wrapper: createWrapper() },
    )

    const textarea = screen.getByPlaceholderText(/Type a new request/)

    // Type @ to trigger autocomplete
    await user.type(textarea, '@But')

    // Wait for autocomplete results to appear
    await waitFor(() => {
      expect(screen.getByText('src/components/Button.tsx')).toBeInTheDocument()
      expect(screen.getByText('src/utils/helpers.ts')).toBeInTheDocument()
    })
  })

  it.skip('should fail to search files when repositoryId is missing in task context - FIXME: Autocomplete component testing issues (TASK-101)', async () => {
    const user = userEvent.setup()
    const taskId = 'task-123'

    // Mock task WITHOUT repositoryId
    const mockTask = {
      iterations: [],
      // repositoryId is missing!
    }

    render(
      <ChatInput
        taskId={taskId}
        task={mockTask as any}
        taskStatus="open"
        onSendMessage={vi.fn()}
      />,
      { wrapper: createWrapper() },
    )

    const textarea = screen.getByPlaceholderText(/Type a new request/)

    // Type @ to trigger autocomplete
    await user.type(textarea, '@But')

    // Verify task search was called but with empty repositoryId
    await waitFor(() => {
      expect(mockTaskFileSearch).toHaveBeenCalledWith(
        taskId,
        'But',
        '', // Empty repositoryId!
        expect.objectContaining({
          enabled: false, // Should be disabled without repositoryId
        }),
      )
    })
  })

  it.skip('should complete file path when selecting from autocomplete - FIXME: Autocomplete component testing issues (TASK-101)', async () => {
    const user = userEvent.setup()
    const taskId = 'task-123'
    const repositoryId = 'repo-456'
    const onSendMessage = vi.fn()

    // Mock successful search response
    mockTaskFileSearch.mockReturnValue({
      data: mockFileSearchResponse,
      isLoading: false,
      error: null,
    } as any)

    const mockTask = {
      iterations: [],
      repositoryId,
    }

    render(
      <ChatInput
        taskId={taskId}
        task={mockTask as any}
        taskStatus="open"
        onSendMessage={onSendMessage}
      />,
      { wrapper: createWrapper() },
    )

    const textarea = screen.getByPlaceholderText(/Type a new request/) as HTMLTextAreaElement

    // Type @ to trigger autocomplete
    await user.type(textarea, 'Check the @But')

    // Wait for results
    await waitFor(() => {
      expect(screen.getByText('src/components/Button.tsx')).toBeInTheDocument()
    })

    // Click on the first result
    const firstResult = screen.getByText('src/components/Button.tsx')
    await user.click(firstResult)

    // Verify the textarea value was updated correctly
    await waitFor(() => {
      expect(textarea.value).toBe('Check the @src/components/Button.tsx')
    })
  })
})
