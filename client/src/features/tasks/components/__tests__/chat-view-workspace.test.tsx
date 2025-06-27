import { EventType } from '@shared/types/event'
import { WorkspaceStatus } from '@shared/types/workspace'
import { screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useTaskEventsWithStreaming } from '@/features/tasks/hooks/use-task-events'
import { useTask, useTaskWorkspaceStatus } from '@/features/tasks/hooks/use-tasks'
import { createMockEvent, createMockTaskWithIterations } from '@/test-utils/factories'
import { renderWithRouter } from '@/test-utils/render'

import { ChatView } from '../chat-view'

// Mock dependencies
vi.mock('@/features/tasks/hooks/use-tasks')
vi.mock('@/features/tasks/hooks/use-task-events')
vi.mock('@tanstack/react-router', () => ({
  useParams: () => ({ taskId: 'task-123' }),
}))
vi.mock('@/features/tasks/stores/task-ui-store', () => ({
  useTaskUIStore: vi.fn((selector) => {
    const state = {
      setSelectedIteration: vi.fn(),
    }
    return selector ? selector(state) : state
  }),
}))

// Mock additional dependencies
vi.mock('@/lib/components/ui/scroll-area', () => ({
  ScrollArea: ({ children, ...props }: any) => <div {...props}>{children}</div>,
}))
vi.mock('../chat-input', () => ({
  ChatInput: () => <div data-testid="task-chat-input">Mock Chat Input</div>,
}))
vi.mock('@/lib/components/error-boundary', () => ({
  ErrorBoundary: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}))
vi.mock('react-markdown', () => ({
  default: ({ children }: { children: string }) => <div>{children}</div>,
}))

describe('ChatView Workspace Integration', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    Element.prototype.scrollIntoView = vi.fn()
  })

  it('should show processing when workspace is running', () => {
    const task = createMockTaskWithIterations(
      {
        task_id: 'task-123',
        is_running: false, // Task itself is not running
        status: 'completed',
      },
      1,
    )

    vi.mocked(useTask).mockReturnValue({
      data: task,
      isLoading: false,
      error: null,
    } as any)

    // Workspace is running
    vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
      data: {
        status: WorkspaceStatus.RUNNING,
        message: 'Processing task',
        created_at: new Date().toISOString(),
      },
      isLoading: false,
      error: null,
    } as any)

    vi.mocked(useTaskEventsWithStreaming).mockReturnValue({
      events: [],
      isLoading: false,
      error: null,
      isStreaming: false,
    })

    renderWithRouter(<ChatView />)

    // Should show processing indicator even though task is not running
    expect(screen.getByText('Processing...')).toBeInTheDocument()
  })

  it('should show processing when workspace is setting up', () => {
    const task = createMockTaskWithIterations(
      {
        task_id: 'task-123',
        is_running: false,
        status: 'completed',
      },
      1,
    )

    vi.mocked(useTask).mockReturnValue({
      data: task,
      isLoading: false,
      error: null,
    } as any)

    vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
      data: {
        status: WorkspaceStatus.SETTING_UP,
        message: 'Setting up workspace',
        created_at: new Date().toISOString(),
      },
      isLoading: false,
      error: null,
    } as any)

    vi.mocked(useTaskEventsWithStreaming).mockReturnValue({
      events: [],
      isLoading: false,
      error: null,
      isStreaming: false,
    })

    renderWithRouter(<ChatView />)

    expect(screen.getByText('Processing...')).toBeInTheDocument()
  })

  it('should update message count from event stream', () => {
    const task = createMockTaskWithIterations(
      {
        task_id: 'task-123',
        is_running: true,
        status: 'active',
      },
      1,
    )
    // Static count is 3
    task.iterations[0].message_count = 3

    const events = [
      createMockEvent({
        type: EventType.MESSAGE,
        iterationId: 1,
        payload: { role: 'assistant', content: 'Message 1' },
      }),
      createMockEvent({
        type: EventType.MESSAGE,
        iterationId: 1,
        payload: { role: 'assistant', content: 'Message 2' },
      }),
      createMockEvent({
        type: EventType.MESSAGE,
        iterationId: 1,
        payload: { role: 'user', content: 'User message' }, // Should not count
      }),
      createMockEvent({
        type: EventType.MESSAGE,
        iterationId: 1,
        payload: { role: 'assistant', content: 'Message 3' },
      }),
      createMockEvent({
        type: EventType.MESSAGE,
        iterationId: 1,
        payload: { role: 'assistant', content: 'Message 4' },
      }),
      createMockEvent({
        type: EventType.MESSAGE,
        iterationId: 1,
        payload: { role: 'assistant', content: 'Message 5' },
      }),
    ]

    vi.mocked(useTask).mockReturnValue({
      data: task,
      isLoading: false,
      error: null,
    } as any)

    vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
      data: {
        status: WorkspaceStatus.RUNNING,
        message: 'Running',
        created_at: new Date().toISOString(),
      },
      isLoading: false,
      error: null,
    } as any)

    vi.mocked(useTaskEventsWithStreaming).mockReturnValue({
      events,
      isLoading: false,
      error: null,
      isStreaming: true,
    })

    renderWithRouter(<ChatView />)

    // Should show 5 messages (from events) instead of 3 (static count)
    expect(screen.getByText('5 messages')).toBeInTheDocument()
  })

  it('should show running clock for active iteration', async () => {
    const createdAt = new Date(Date.now() - 65000) // 65 seconds ago
    const task = createMockTaskWithIterations(
      {
        task_id: 'task-123',
        is_running: true,
        status: 'active',
      },
      1,
    )
    task.iterations[0].start_time = createdAt.toISOString()
    task.iterations[0].duration_ms = 10000 // Static duration should be ignored

    vi.mocked(useTask).mockReturnValue({
      data: task,
      isLoading: false,
      error: null,
    } as any)

    vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
      data: {
        status: WorkspaceStatus.RUNNING,
        message: 'Running',
        created_at: new Date().toISOString(),
      },
      isLoading: false,
      error: null,
    } as any)

    vi.mocked(useTaskEventsWithStreaming).mockReturnValue({
      events: [],
      isLoading: false,
      error: null,
      isStreaming: false,
    })

    renderWithRouter(<ChatView />)

    // Should show time elapsed since creation (approximately 1m 5s)
    await waitFor(() => {
      const durationText = screen.getByText(/\d+m \d+s/)
      expect(durationText).toBeInTheDocument()
      // Check it's showing at least 1 minute
      expect(durationText.textContent).toMatch(/[1-9]\d*m/)
    })
  })

  it('should use static values when not streaming', () => {
    const task = createMockTaskWithIterations(
      {
        task_id: 'task-123',
        is_running: false,
        status: 'completed',
      },
      1,
    )
    task.iterations[0].message_count = 10
    task.iterations[0].duration_ms = 30000 // 30 seconds

    vi.mocked(useTask).mockReturnValue({
      data: task,
      isLoading: false,
      error: null,
    } as any)

    vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
      data: {
        status: WorkspaceStatus.IDLE,
        message: 'Idle',
        created_at: new Date().toISOString(),
      },
      isLoading: false,
      error: null,
    } as any)

    vi.mocked(useTaskEventsWithStreaming).mockReturnValue({
      events: [],
      isLoading: false,
      error: null,
      isStreaming: false,
    })

    renderWithRouter(<ChatView />)

    // Should use static values
    expect(screen.getByText('10 messages')).toBeInTheDocument()
    expect(screen.getByText('0m 30s')).toBeInTheDocument()
  })
})
