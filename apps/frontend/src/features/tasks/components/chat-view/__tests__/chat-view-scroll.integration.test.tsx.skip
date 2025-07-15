import { render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  useEventStream,
  useStreamingStatus,
  useTaskData,
  useWorkspaceStatus,
} from '@/features/tasks/contexts/task-providers'
import { useSelectedIteration } from '@/features/tasks/contexts/task-ui-provider'
import { useSendMessage, useTaskCommands } from '@/features/tasks/hooks/use-tasks'
import { ChatView } from '../chat-view'

// Mock all dependencies
vi.mock('@/features/tasks/contexts/task-providers')
vi.mock('@/features/tasks/contexts/task-ui-provider')
vi.mock('@/features/tasks/hooks/use-tasks')
vi.mock('@/features/tasks/hooks/use-auto-scroll', () => ({
  useAutoScroll: () => ({ scrollRef: { current: null } }),
}))
vi.mock('@tanstack/react-router', () => ({
  useParams: () => ({ taskId: 'test-task-id' }),
}))

// Mock components that aren't relevant to scroll testing
vi.mock('../chat-header', () => ({
  ChatHeader: () => <div>Chat Header</div>,
}))
vi.mock('../chat-view-skeleton', () => ({
  ChatViewSkeleton: () => <div>Loading...</div>,
}))
vi.mock('../../event-viewer', () => ({
  TaskMinifiedLastEventsDisplay: () => <div>Events</div>,
}))
vi.mock('../../chat-input', () => ({
  ChatInput: () => <div>Chat Input</div>,
}))
vi.mock('../command-buttons', () => ({
  CommandButtons: () => <div>Commands</div>,
}))

describe('ChatView Scroll Behavior', () => {
  beforeEach(() => {
    vi.clearAllMocks()

    // Mock scrollIntoView
    Element.prototype.scrollIntoView = vi.fn()

    // Default mocks
    vi.mocked(useTaskData).mockReturnValue({
      taskId: 'test-task-id',
      status: 'open',
      iterations: [
        {
          iterationNumber: 1,
          initialMessage: 'Test message',
          result: 'Test result',
          status: 'completed',
          messageCount: 1,
          costUsd: 0.01,
          durationMs: 1000,
          startTime: new Date().toISOString(),
        },
      ],
      repositoryId: 'test-repo',
      isLoading: false,
    } as any)

    vi.mocked(useEventStream).mockReturnValue({
      events: [],
    } as any)

    vi.mocked(useStreamingStatus).mockReturnValue({
      isStreaming: false,
    } as any)

    vi.mocked(useSelectedIteration).mockReturnValue({
      setSelectedIteration: vi.fn(),
    } as any)

    vi.mocked(useSendMessage).mockReturnValue({
      mutate: vi.fn(),
      isPending: false,
    } as any)

    vi.mocked(useWorkspaceStatus).mockReturnValue({
      workspaceHasChanges: false,
    } as any)

    vi.mocked(useTaskCommands).mockReturnValue({
      data: { commands: [] },
      isLoading: false,
    } as any)
  })

  it('should render without crashing with new scroll implementation', () => {
    render(<ChatView />)

    expect(screen.getByText('Chat Header')).toBeInTheDocument()
    expect(screen.getByText('Test message')).toBeInTheDocument()
    expect(screen.getByText('Test result')).toBeInTheDocument()
    expect(screen.getByText('Chat Input')).toBeInTheDocument()
  })

  it('should not show scroll to bottom button initially', () => {
    render(<ChatView />)

    expect(screen.queryByText('Scroll to bottom')).not.toBeInTheDocument()
  })

  it('should handle loading state correctly', () => {
    vi.mocked(useTaskData).mockReturnValue({
      taskId: null,
      status: 'open',
      iterations: [],
      repositoryId: null,
      isLoading: true,
    } as any)

    render(<ChatView />)

    expect(screen.getByText('Loading...')).toBeInTheDocument()
  })
})
