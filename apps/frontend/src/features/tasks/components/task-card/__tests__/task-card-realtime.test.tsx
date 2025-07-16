import type { Event, Task } from '@moatless/types'
import { EventType } from '@moatless/types'
import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import * as globalEventStreamModule from '@/features/events/contexts/global-event-stream-provider'
import { TaskCard } from '../task-card'

// Mock the event stream provider
vi.mock('@/features/events/contexts/global-event-stream-provider', () => ({
  useTaskEventSubscription: vi.fn(),
}))

// Mock the hooks
vi.mock('@/features/tasks/hooks/use-tasks', () => ({
  useTaskEvents: vi.fn(() => ({
    data: [
      {
        eventId: 'initial-1',
        taskId: 'task-123',
        type: EventType.MESSAGE,
        payload: { type: 'assistant', content: 'Initial message' },
        createdAt: new Date().toISOString(),
      },
    ],
    isLoading: false,
    error: null,
  })),
}))

// Mock UI components
vi.mock('@/features/tasks/components/minified-event-viewer/minified-event-item', () => ({
  MinifiedEventItem: ({ event }: { event: Event }) => (
    <div data-testid="event-item">{event.eventId}</div>
  ),
}))

vi.mock('@/features/tasks/components/task-status-icon', () => ({
  TaskStatusIcon: () => <div data-testid="status-icon">Running</div>,
}))

// Mock router
vi.mock('@tanstack/react-router', () => ({
  Link: ({ children, ...props }: any) => <a {...props}>{children}</a>,
}))

describe('TaskCard Real-time Updates', () => {
  const mockTask: Task = {
    taskId: 'task-123',
    repositoryId: 'repo-123',
    agentType: 'test-agent',
    status: 'running',
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
    lastMessageAt: new Date().toISOString(),
    iterations: [
      {
        iterationId: 'iter-1',
        taskId: 'task-123',
        iterationNumber: 1,
        initialMessage: 'Test task',
        startTime: new Date().toISOString(),
        status: 'running',
      },
    ],
  }

  it('should display real-time events as they arrive', () => {
    let eventHandler: ((event: Event) => void) | null = null

    // Capture the event handler when subscribed
    vi.mocked(globalEventStreamModule.useTaskEventSubscription).mockImplementation(
      (taskId, handler) => {
        eventHandler = handler
      },
    )

    const { rerender } = render(<TaskCard task={mockTask} repositoryName="Test Repo" />)

    // Initially should show 1 event from the mocked data
    expect(screen.getAllByTestId('event-item')).toHaveLength(1)
    expect(screen.getByText('initial-1')).toBeInTheDocument()

    // Simulate receiving a new event
    const newEvent: Event = {
      eventId: 'realtime-1',
      taskId: 'task-123',
      type: EventType.MESSAGE,
      payload: { type: 'assistant', content: 'Real-time message' },
      createdAt: new Date().toISOString(),
    }

    if (eventHandler) {
      eventHandler(newEvent)
    }

    // Force re-render to see the updated state
    rerender(<TaskCard task={mockTask} repositoryName="Test Repo" />)

    // Should now show 2 events (initial + real-time)
    const eventItems = screen.getAllByTestId('event-item')
    expect(eventItems).toHaveLength(2)
    expect(screen.getByText('initial-1')).toBeInTheDocument()
    expect(screen.getByText('realtime-1')).toBeInTheDocument()
  })

  it('should only show last 3 events', () => {
    let eventHandler: ((event: Event) => void) | null = null

    vi.mocked(globalEventStreamModule.useTaskEventSubscription).mockImplementation(
      (taskId, handler) => {
        eventHandler = handler
      },
    )

    const { rerender } = render(<TaskCard task={mockTask} repositoryName="Test Repo" />)

    // Simulate receiving 4 new events
    const newEvents: Event[] = Array.from({ length: 4 }, (_, i) => ({
      eventId: `realtime-${i + 1}`,
      taskId: 'task-123',
      type: EventType.MESSAGE,
      payload: { type: 'assistant', content: `Message ${i + 1}` },
      createdAt: new Date().toISOString(),
    }))

    newEvents.forEach((event) => {
      if (eventHandler) {
        eventHandler(event)
      }
    })

    rerender(<TaskCard task={mockTask} repositoryName="Test Repo" />)

    // Should only show last 3 events (initial is replaced by newer events)
    const eventItems = screen.getAllByTestId('event-item')
    expect(eventItems).toHaveLength(3)
    expect(screen.getByText('realtime-2')).toBeInTheDocument()
    expect(screen.getByText('realtime-3')).toBeInTheDocument()
    expect(screen.getByText('realtime-4')).toBeInTheDocument()
    // First events should not be visible
    expect(screen.queryByText('initial-1')).not.toBeInTheDocument()
    expect(screen.queryByText('realtime-1')).not.toBeInTheDocument()
  })

  it('should clear real-time events when task stops running', () => {
    let eventHandler: ((event: Event) => void) | null = null

    vi.mocked(globalEventStreamModule.useTaskEventSubscription).mockImplementation(
      (taskId, handler) => {
        eventHandler = handler
      },
    )

    const { rerender } = render(<TaskCard task={mockTask} repositoryName="Test Repo" />)

    // Add a real-time event
    const newEvent: Event = {
      eventId: 'realtime-1',
      taskId: 'task-123',
      type: EventType.MESSAGE,
      payload: { type: 'assistant', content: 'Real-time message' },
      createdAt: new Date().toISOString(),
    }

    if (eventHandler) {
      eventHandler(newEvent)
    }

    rerender(<TaskCard task={mockTask} repositoryName="Test Repo" />)
    expect(screen.getAllByTestId('event-item')).toHaveLength(2)

    // Change task status to completed
    const completedTask = { ...mockTask, status: 'completed' as const }
    rerender(<TaskCard task={completedTask} repositoryName="Test Repo" />)

    // Should not show events anymore (task is not running)
    expect(screen.queryByTestId('event-item')).not.toBeInTheDocument()
  })

  it('should not add duplicate events', () => {
    let eventHandler: ((event: Event) => void) | null = null

    vi.mocked(globalEventStreamModule.useTaskEventSubscription).mockImplementation(
      (taskId, handler) => {
        eventHandler = handler
      },
    )

    const { rerender } = render(<TaskCard task={mockTask} repositoryName="Test Repo" />)

    // Send the same event twice
    const duplicateEvent: Event = {
      eventId: 'duplicate-1',
      taskId: 'task-123',
      type: EventType.MESSAGE,
      payload: { type: 'assistant', content: 'Duplicate message' },
      createdAt: new Date().toISOString(),
    }

    if (eventHandler) {
      eventHandler(duplicateEvent)
      eventHandler(duplicateEvent) // Send again
    }

    rerender(<TaskCard task={mockTask} repositoryName="Test Repo" />)

    // Should only show the event once
    const eventItems = screen.getAllByTestId('event-item')
    expect(eventItems).toHaveLength(2) // initial + 1 duplicate (not 3)
    expect(screen.getAllByText('duplicate-1')).toHaveLength(1)
  })
})
