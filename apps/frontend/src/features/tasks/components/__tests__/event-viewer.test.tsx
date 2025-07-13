import type { Event } from '@moatless/shared/types/event'
import { act, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createMockEventList } from '@/test-utils/factories'
import { renderWithRouter } from '@/test-utils/render'

import { EventViewer } from '../event-viewer/event-viewer'

// Mock hooks - need to use hoisted variables
vi.mock('../../contexts/task-providers', () => ({
  useTaskEvents: vi.fn(() => ({
    events: [] as Array<Event>,
    streamingEvents: [] as Array<Event>,
    isStreaming: false,
    streamingError: null,
    latestEventId: null,
    hasNewEvents: false,
    error: null as Error | null,
    isLoading: false,
  })),
  useTaskLoadingError: vi.fn(() => ({
    isLoading: false,
    error: null as Error | null,
  })),
}))

// Top-level regex for event test IDs
const EVENT_TESTID_REGEX = /^event-\d+$/

// Mock dependencies
// Note: use-task-events hook was removed during SSE migration
vi.mock('@tanstack/react-router', () => ({
  useParams: () => ({ taskId: 'task-123' }),
}))

interface MockScrollAreaProps {
  children?: React.ReactNode
  ref?: React.Ref<HTMLDivElement>
  className?: string
}

// Mock ScrollArea to simplify testing
vi.mock('@/lib/components/ui/scroll-area', () => ({
  ScrollArea: vi.fn().mockImplementation(({ children, ...props }: MockScrollAreaProps) => (
    <div ref={props.ref} data-testid="scroll-area">
      <div data-slot="scroll-area-viewport" style={{ overflowY: 'auto' }}>
        {children}
      </div>
    </div>
  )),
}))

// Mock EventList
vi.mock('../event-viewer/event-list', () => ({
  EventList: ({ events }: { events: Array<Event> }) => (
    <div data-testid="event-list">
      {events.map((event, index) => (
        <div key={event.eventId || index} data-testid={`event-${index}`}>
          Event {index}
        </div>
      ))}
    </div>
  ),
}))

// Import mocked module
import { useTaskEvents, useTaskLoadingError } from '../../contexts/task-providers'

// Helper to create default useTaskEvents return value
const createDefaultTaskEventsReturn = (overrides = {}) => ({
  events: [] as Array<Event>,
  streamingEvents: [] as Array<Event>,
  isStreaming: false,
  streamingError: null,
  latestEventId: null,
  hasNewEvents: false,
  error: null as Error | null,
  isLoading: false,
  ...overrides,
})

describe('EventViewer', () => {
  // Clear visited tasks before each test
  beforeEach(() => {
    vi.clearAllMocks()
  })

  afterEach(() => {
    vi.clearAllTimers()
  })

  describe('Loading States', () => {
    it('shows loading state', () => {
      vi.mocked(useTaskEvents).mockReturnValue(createDefaultTaskEventsReturn({ isLoading: true }))
      vi.mocked(useTaskLoadingError).mockReturnValue({
        isLoading: true,
        error: null,
      })

      renderWithRouter(<EventViewer />)

      expect(screen.getByText('Loading events...')).toBeInTheDocument()
      // SVG spinner doesn't have role="img", check for the Loader2 icon instead
      const spinner = document.querySelector('.lucide-loader-circle')
      expect(spinner).toHaveClass('animate-spin')
    })

    it('shows error state', () => {
      const error = new Error('Failed to fetch events')
      vi.mocked(useTaskEvents).mockReturnValue(createDefaultTaskEventsReturn({ error }))
      vi.mocked(useTaskLoadingError).mockReturnValue({
        isLoading: false,
        error,
      })

      renderWithRouter(<EventViewer />)

      expect(screen.getByText('Failed to load events')).toBeInTheDocument()
      expect(screen.getByText('Failed to fetch events')).toBeInTheDocument()
    })

    it('shows empty state when no events', () => {
      vi.mocked(useTaskEvents).mockReturnValue(createDefaultTaskEventsReturn())
      vi.mocked(useTaskLoadingError).mockReturnValue({
        isLoading: false,
        error: null,
      })

      renderWithRouter(<EventViewer />)

      expect(screen.getByText('No events yet')).toBeInTheDocument()
    })

    it('shows waiting state when streaming with no events', () => {
      vi.mocked(useTaskEvents).mockReturnValue(createDefaultTaskEventsReturn({ isStreaming: true }))
      vi.mocked(useTaskLoadingError).mockReturnValue({
        isLoading: false,
        error: null,
      })

      renderWithRouter(<EventViewer />)

      // When streaming with no events, it shows the same empty state
      expect(screen.getByText('No events yet')).toBeInTheDocument()
    })
  })

  describe('Event Display', () => {
    it('renders event list when events are available', () => {
      const mockEvents = createMockEventList(5)
      vi.mocked(useTaskEvents).mockReturnValue(
        createDefaultTaskEventsReturn({
          events: mockEvents,
        }),
      )
      vi.mocked(useTaskLoadingError).mockReturnValue({
        isLoading: false,
        error: null,
      })

      renderWithRouter(<EventViewer />)

      expect(screen.getByTestId('event-list')).toBeInTheDocument()
      // The regex /^event-/ matches event-list as well, so be more specific
      expect(screen.getAllByTestId(EVENT_TESTID_REGEX)).toHaveLength(5)
    })

    it('updates when new events arrive', () => {
      const initialEvents = createMockEventList(3)

      vi.mocked(useTaskEvents).mockReturnValue(
        createDefaultTaskEventsReturn({
          events: initialEvents,
          isStreaming: true,
        }),
      )
      vi.mocked(useTaskLoadingError).mockReturnValue({
        isLoading: false,
        error: null,
      })

      const { rerender } = renderWithRouter(<EventViewer />)
      expect(screen.getAllByTestId(/^event-\d+$/)).toHaveLength(3)

      // Add more events
      const updatedEvents = createMockEventList(5)
      vi.mocked(useTaskEvents).mockReturnValue(
        createDefaultTaskEventsReturn({
          events: updatedEvents,
          isStreaming: true,
        }),
      )

      rerender(<EventViewer />)
      expect(screen.getAllByTestId(EVENT_TESTID_REGEX)).toHaveLength(5)
    })
  })

  describe('Auto-scrolling Behavior', () => {
    let scrollElement: HTMLElement
    let requestAnimationFrameSpy: any

    beforeEach(() => {
      // Mock requestAnimationFrame
      requestAnimationFrameSpy = vi
        .spyOn(window, 'requestAnimationFrame')
        .mockImplementation((cb: FrameRequestCallback) => {
          cb(0)
          return 1
        })
    })

    it.skip('scrolls to bottom on first visit - FIXME: Module state reset needed (TASK-001)', () => {
      // This test depends on module-level state that's hard to reset
      // Need to refactor visitedTasks to be testable
    })

    it.skip('does not auto-scroll on subsequent visits to same task - FIXME: Module state reset needed (TASK-001)', () => {
      // This test depends on module-level state that's hard to reset
      // Need to refactor visitedTasks to be testable
    })

    it.skip('auto-scrolls when new events arrive if at bottom - FIXME: useEffect timing issues (TASK-002)', () => {
      // This test has timing issues with useEffect
      // Need to use proper async testing utilities
    })

    it('does not auto-scroll when not at bottom', async () => {
      const initialEvents = createMockEventList(5)
      vi.mocked(useTaskEvents).mockReturnValue(
        createDefaultTaskEventsReturn({
          events: initialEvents,
          isStreaming: true,
        }),
      )

      const { rerender } = renderWithRouter(<EventViewer />)

      await waitFor(() => {
        scrollElement = screen
          .getByTestId('scroll-area')
          .querySelector('[data-slot="scroll-area-viewport"]') as HTMLElement
      })

      // Simulate NOT being at bottom
      Object.defineProperty(scrollElement, 'scrollTop', {
        writable: true,
        value: 50,
      })
      Object.defineProperty(scrollElement, 'scrollHeight', {
        writable: true,
        value: 200,
      })
      Object.defineProperty(scrollElement, 'clientHeight', {
        writable: true,
        value: 100,
      })

      // Fire scroll event to update isAtBottom state
      scrollElement.dispatchEvent(new window.Event('scroll'))

      // Clear previous calls
      requestAnimationFrameSpy.mockClear()

      // Add new events
      const updatedEvents = createMockEventList(8)
      vi.mocked(useTaskEvents).mockReturnValue(
        createDefaultTaskEventsReturn({
          events: updatedEvents,
          isStreaming: true,
        }),
      )

      rerender(<EventViewer />)

      // Should not scroll
      expect(requestAnimationFrameSpy).not.toHaveBeenCalled()
    })
  })

  describe('Scroll Event Handling', () => {
    it('tracks scroll position', () => {
      const mockEvents = createMockEventList(10)
      vi.mocked(useTaskEvents).mockReturnValue(
        createDefaultTaskEventsReturn({
          events: mockEvents,
        }),
      )

      renderWithRouter(<EventViewer />)

      const scrollElement = screen
        .getByTestId('scroll-area')
        .querySelector('[data-slot="scroll-area-viewport"]') as HTMLElement

      // Mock scroll properties
      Object.defineProperty(scrollElement, 'scrollTop', {
        writable: true,
        value: 0,
      })
      Object.defineProperty(scrollElement, 'scrollHeight', {
        writable: true,
        value: 200,
      })
      Object.defineProperty(scrollElement, 'clientHeight', {
        writable: true,
        value: 100,
      })

      // Simulate scroll event
      act(() => {
        scrollElement.dispatchEvent(new window.Event('scroll'))
      })

      // Component should track that we're not at bottom
      // (Internal state, hard to test directly without exposing it)
    })

    it('cleans up scroll event listener on unmount', () => {
      const mockEvents = createMockEventList(5)
      vi.mocked(useTaskEvents).mockReturnValue(
        createDefaultTaskEventsReturn({
          events: mockEvents,
        }),
      )

      const { unmount } = renderWithRouter(<EventViewer />)

      const scrollElement = screen
        .getByTestId('scroll-area')
        .querySelector('[data-slot="scroll-area-viewport"]') as HTMLElement

      const removeEventListenerSpy = vi.spyOn(scrollElement, 'removeEventListener')

      unmount()

      expect(removeEventListenerSpy).toHaveBeenCalledWith('scroll', expect.any(Function))
    })
  })

  describe('Edge Cases', () => {
    it('handles missing scroll element gracefully', async () => {
      vi.mocked(useTaskEvents).mockReturnValue(
        createDefaultTaskEventsReturn({
          events: createMockEventList(5),
        }),
      )

      // Mock ScrollArea to not have the expected structure
      const { ScrollArea } = await import('@/lib/components/ui/scroll-area')
      vi.mocked(ScrollArea).mockImplementationOnce(({ children }: MockScrollAreaProps) => (
        <div>{children}</div>
      ))

      expect(() => {
        renderWithRouter(<EventViewer />)
      }).not.toThrow()
    })

    it('handles empty events array without error', () => {
      vi.mocked(useTaskEvents).mockReturnValue(createDefaultTaskEventsReturn())

      renderWithRouter(<EventViewer />)

      expect(screen.getByText('No events yet')).toBeInTheDocument()
    })

    it('does not scroll when events length stays the same', async () => {
      const mockEvents = createMockEventList(5)
      vi.mocked(useTaskEvents).mockReturnValue(
        createDefaultTaskEventsReturn({
          events: mockEvents,
          isStreaming: true,
        }),
      )

      const { rerender } = renderWithRouter(<EventViewer />)

      await waitFor(() => {
        expect(screen.getByTestId('event-list')).toBeInTheDocument()
      })

      const requestAnimationFrameSpy = vi
        .spyOn(window, 'requestAnimationFrame')
        .mockImplementation((cb: FrameRequestCallback) => {
          cb(0)
          return 1
        })

      // Rerender with same events
      rerender(<EventViewer />)

      // Should not trigger new scroll
      expect(requestAnimationFrameSpy).not.toHaveBeenCalled()
    })
  })
})
