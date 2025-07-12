import { act, render, screen } from '@testing-library/react'
import React from 'react'
import { describe, expect, it, vi } from 'vitest'
import { TaskUIProvider, useChatState } from '@/features/tasks/contexts/task-ui-provider'

// Mock scroll functionality
const mockScrollIntoView = vi.fn()
Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
  configurable: true,
  value: mockScrollIntoView,
})

// Mock querySelector for scroll area
const mockQuerySelector = vi.fn()
Object.defineProperty(document, 'querySelector', {
  configurable: true,
  value: mockQuerySelector,
})

// Test component that simulates chat scroll behavior
function ChatScrollTestComponent() {
  const { chatState, updateChatState } = useChatState()
  const [scrollPosition, setScrollPosition] = React.useState(0)

  // Simulate scroll event handler
  const handleScroll = (newPosition: number) => {
    setScrollPosition(newPosition)
    const isAtBottom = newPosition > 800 // Simulate "at bottom" logic
    updateChatState({
      isAtBottom: isAtBottom, // use actual value
      scrollPosition: newPosition,
    })
  }

  return (
    <div>
      <div data-testid="chat-is-at-bottom">{chatState.isAtBottom.toString()}</div>
      <div data-testid="chat-scroll-position">{chatState.scrollPosition || 0}</div>
      <div data-testid="current-scroll">{scrollPosition}</div>

      <button type="button" data-testid="scroll-to-top" onClick={() => handleScroll(0)}>
        Scroll to Top
      </button>

      <button type="button" data-testid="scroll-to-middle" onClick={() => handleScroll(500)}>
        Scroll to Middle
      </button>

      <button type="button" data-testid="scroll-to-bottom" onClick={() => handleScroll(1000)}>
        Scroll to Bottom
      </button>

      <button
        type="button"
        data-testid="mark-at-bottom"
        onClick={() => updateChatState({ isAtBottom: true })}
      >
        Mark At Bottom
      </button>
    </div>
  )
}

// Multi-task component for testing isolation
function MultiTaskChatTest() {
  const [currentTaskId, setCurrentTaskId] = React.useState('chat-task-1')

  return (
    <div>
      <div data-testid="current-task-id">{currentTaskId}</div>

      <button
        type="button"
        data-testid="switch-to-task-1"
        onClick={() => setCurrentTaskId('chat-task-1')}
      >
        Switch to Task 1
      </button>

      <button
        type="button"
        data-testid="switch-to-task-2"
        onClick={() => setCurrentTaskId('chat-task-2')}
      >
        Switch to Task 2
      </button>

      <button
        type="button"
        data-testid="switch-to-task-3"
        onClick={() => setCurrentTaskId('chat-task-3')}
      >
        Switch to Task 3
      </button>

      <TaskUIProvider taskId={currentTaskId}>
        <ChatScrollTestComponent />
      </TaskUIProvider>
    </div>
  )
}

describe('Chat State Isolation', () => {
  it('should maintain default chat state for new tasks', () => {
    render(
      <TaskUIProvider taskId="new-chat-task">
        <ChatScrollTestComponent />
      </TaskUIProvider>,
    )

    expect(screen.getByTestId('chat-is-at-bottom')).toHaveTextContent('true')
    expect(screen.getByTestId('chat-scroll-position')).toHaveTextContent('0')
  })

  it('should update chat state within a single task', () => {
    render(
      <TaskUIProvider taskId="single-chat-task">
        <ChatScrollTestComponent />
      </TaskUIProvider>,
    )

    // Initial state
    expect(screen.getByTestId('chat-is-at-bottom')).toHaveTextContent('true')

    // Scroll to middle (500 < 800, so not at bottom)
    act(() => {
      screen.getByTestId('scroll-to-middle').click()
    })

    expect(screen.getByTestId('chat-is-at-bottom')).toHaveTextContent('false')
    expect(screen.getByTestId('chat-scroll-position')).toHaveTextContent('500')

    // Scroll to bottom (1000 > 800, so at bottom)
    act(() => {
      screen.getByTestId('scroll-to-bottom').click()
    })

    expect(screen.getByTestId('chat-is-at-bottom')).toHaveTextContent('true')
    expect(screen.getByTestId('chat-scroll-position')).toHaveTextContent('1000')
  })

  it('should isolate chat scroll state between different tasks', () => {
    render(<MultiTaskChatTest />)

    // Task 1: Scroll to middle (500 < 800, so not at bottom)
    expect(screen.getByTestId('current-task-id')).toHaveTextContent('chat-task-1')

    act(() => {
      screen.getByTestId('scroll-to-middle').click()
    })

    expect(screen.getByTestId('chat-is-at-bottom')).toHaveTextContent('false')
    expect(screen.getByTestId('chat-scroll-position')).toHaveTextContent('500')

    // Switch to Task 2 (should have default state)
    act(() => {
      screen.getByTestId('switch-to-task-2').click()
    })

    expect(screen.getByTestId('current-task-id')).toHaveTextContent('chat-task-2')
    expect(screen.getByTestId('chat-is-at-bottom')).toHaveTextContent('true') // default
    expect(screen.getByTestId('chat-scroll-position')).toHaveTextContent('0') // default

    // Task 2: Scroll to top (0 < 800, so not at bottom)
    act(() => {
      screen.getByTestId('scroll-to-top').click()
    })

    expect(screen.getByTestId('chat-is-at-bottom')).toHaveTextContent('false')
    expect(screen.getByTestId('chat-scroll-position')).toHaveTextContent('0')

    // Switch back to Task 1 (should restore previous state)
    act(() => {
      screen.getByTestId('switch-to-task-1').click()
    })

    expect(screen.getByTestId('current-task-id')).toHaveTextContent('chat-task-1')
    expect(screen.getByTestId('chat-is-at-bottom')).toHaveTextContent('false') // preserved
    expect(screen.getByTestId('chat-scroll-position')).toHaveTextContent('500') // preserved

    // Switch to Task 3 (fresh task, default state)
    act(() => {
      screen.getByTestId('switch-to-task-3').click()
    })

    expect(screen.getByTestId('current-task-id')).toHaveTextContent('chat-task-3')
    expect(screen.getByTestId('chat-is-at-bottom')).toHaveTextContent('true') // default
    expect(screen.getByTestId('chat-scroll-position')).toHaveTextContent('0') // default

    // Verify Task 2 state is still preserved
    act(() => {
      screen.getByTestId('switch-to-task-2').click()
    })

    expect(screen.getByTestId('current-task-id')).toHaveTextContent('chat-task-2')
    expect(screen.getByTestId('chat-is-at-bottom')).toHaveTextContent('false') // preserved
    expect(screen.getByTestId('chat-scroll-position')).toHaveTextContent('0') // preserved
  })

  it('should handle complex scroll state scenarios', () => {
    render(<MultiTaskChatTest />)

    // Task 1: Multiple scroll operations - end at middle position
    act(() => {
      screen.getByTestId('scroll-to-bottom').click() // 1000 > 800 = true
    })
    act(() => {
      screen.getByTestId('scroll-to-middle').click() // 500 < 800 = false
    })

    const task1FinalState = {
      isAtBottom: screen.getByTestId('chat-is-at-bottom').textContent,
      scrollPosition: screen.getByTestId('chat-scroll-position').textContent,
    }

    // Task 1 should be at middle (not at bottom)
    expect(task1FinalState.isAtBottom).toBe('false')
    expect(task1FinalState.scrollPosition).toBe('500')

    // Switch to Task 2 and perform different operations - end at bottom
    act(() => {
      screen.getByTestId('switch-to-task-2').click()
    })

    act(() => {
      screen.getByTestId('scroll-to-top').click() // 0 < 800 = false
    })
    act(() => {
      screen.getByTestId('scroll-to-bottom').click() // 1000 > 800 = true
    })

    const task2FinalState = {
      isAtBottom: screen.getByTestId('chat-is-at-bottom').textContent,
      scrollPosition: screen.getByTestId('chat-scroll-position').textContent,
    }

    // Task 2 should be at bottom
    expect(task2FinalState.isAtBottom).toBe('true')
    expect(task2FinalState.scrollPosition).toBe('1000')

    // Verify states are different
    expect(task1FinalState.isAtBottom).not.toBe(task2FinalState.isAtBottom)
    expect(task1FinalState.scrollPosition).not.toBe(task2FinalState.scrollPosition)

    // Switch back and verify Task 1 state preserved
    act(() => {
      screen.getByTestId('switch-to-task-1').click()
    })

    expect(screen.getByTestId('chat-is-at-bottom')).toHaveTextContent(task1FinalState.isAtBottom!)
    expect(screen.getByTestId('chat-scroll-position')).toHaveTextContent(
      task1FinalState.scrollPosition!,
    )

    // Switch back and verify Task 2 state preserved
    act(() => {
      screen.getByTestId('switch-to-task-2').click()
    })

    expect(screen.getByTestId('chat-is-at-bottom')).toHaveTextContent(task2FinalState.isAtBottom!)
    expect(screen.getByTestId('chat-scroll-position')).toHaveTextContent(
      task2FinalState.scrollPosition!,
    )
  })

  it('should handle rapid task switching without state corruption', () => {
    render(<MultiTaskChatTest />)

    // Set up initial different states for each task step by step
    expect(screen.getByTestId('current-task-id')).toHaveTextContent('chat-task-1')

    act(() => {
      screen.getByTestId('scroll-to-middle').click() // Task 1: 500
    })
    expect(screen.getByTestId('chat-scroll-position')).toHaveTextContent('500')

    act(() => {
      screen.getByTestId('switch-to-task-2').click()
    })
    expect(screen.getByTestId('current-task-id')).toHaveTextContent('chat-task-2')

    act(() => {
      screen.getByTestId('scroll-to-bottom').click() // Task 2: 1000
    })
    expect(screen.getByTestId('chat-scroll-position')).toHaveTextContent('1000')

    act(() => {
      screen.getByTestId('switch-to-task-3').click()
    })
    expect(screen.getByTestId('current-task-id')).toHaveTextContent('chat-task-3')

    act(() => {
      screen.getByTestId('scroll-to-top').click() // Task 3: 0
    })
    expect(screen.getByTestId('chat-scroll-position')).toHaveTextContent('0')

    // Rapidly switch between tasks multiple times to test persistence
    for (let i = 0; i < 3; i++) {
      act(() => {
        screen.getByTestId('switch-to-task-1').click()
      })
      expect(screen.getByTestId('current-task-id')).toHaveTextContent('chat-task-1')
      expect(screen.getByTestId('chat-scroll-position')).toHaveTextContent('500') // should persist

      act(() => {
        screen.getByTestId('switch-to-task-2').click()
      })
      expect(screen.getByTestId('current-task-id')).toHaveTextContent('chat-task-2')
      expect(screen.getByTestId('chat-scroll-position')).toHaveTextContent('1000') // should persist

      act(() => {
        screen.getByTestId('switch-to-task-3').click()
      })
      expect(screen.getByTestId('current-task-id')).toHaveTextContent('chat-task-3')
      expect(screen.getByTestId('chat-scroll-position')).toHaveTextContent('0') // should persist
    }
  })
})
