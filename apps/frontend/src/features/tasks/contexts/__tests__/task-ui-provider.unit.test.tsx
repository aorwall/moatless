import { act, render, screen } from '@testing-library/react'
import React from 'react'
import { describe, expect, it } from 'vitest'
import { TaskUIProvider, useTaskUI } from '@/features/tasks'

// Test component that uses the hook
function TestComponent({ taskId }: { taskId: string }) {
  const { currentState } = useTaskUI()

  return (
    <div>
      <div data-testid="task-id">{taskId}</div>
      <div data-testid="panel-left">{currentState.panelSizes.left}</div>
      <div data-testid="panel-right">{currentState.panelSizes.right}</div>
    </div>
  )
}

describe('TaskUIProvider', () => {
  it('should provide default state for new tasks', () => {
    render(
      <TaskUIProvider taskId="task-1">
        <TestComponent taskId="task-1" />
      </TaskUIProvider>,
    )

    expect(screen.getByTestId('panel-left')).toHaveTextContent('60')
    expect(screen.getByTestId('panel-right')).toHaveTextContent('40')
  })

  it('should throw error when hooks are used outside provider', () => {
    // Component that uses useTaskUI directly
    function ErrorTestComponent() {
      const context = useTaskUI()
      return <div>{context ? 'has context' : 'no context'}</div>
    }

    // Suppress console.error for this test
    const originalError = console.error
    console.error = () => undefined

    expect(() => {
      render(<ErrorTestComponent />)
    }).toThrow('useTaskUI must be used within TaskUIProvider')

    console.error = originalError
  })
})

describe('TaskUIProvider State Isolation', () => {
  function MultiTaskTest() {
    const [currentTaskId, setCurrentTaskId] = React.useState('task-1')

    return (
      <div>
        <div data-testid="active-task-id">{currentTaskId}</div>
        <button
          type="button"
          data-testid="switch-to-task-1"
          onClick={() => setCurrentTaskId('task-1')}
        >
          Task 1
        </button>
        <button
          type="button"
          data-testid="switch-to-task-2"
          onClick={() => setCurrentTaskId('task-2')}
        >
          Task 2
        </button>

        <TaskUIProvider taskId={currentTaskId}>
          <TestComponent taskId={currentTaskId} />
        </TaskUIProvider>
      </div>
    )
  }

  it('should isolate state between different tasks', () => {
    render(<MultiTaskTest />)

    // Verify we start with task-1
    expect(screen.getByTestId('active-task-id')).toHaveTextContent('task-1')
    expect(screen.getByTestId('task-id')).toHaveTextContent('task-1')

    // Initial state should be default
    expect(screen.getByTestId('panel-left')).toHaveTextContent('60')

    // Switch to task-2 (should have default state)
    act(() => {
      screen.getByTestId('switch-to-task-2').click()
    })

    expect(screen.getByTestId('active-task-id')).toHaveTextContent('task-2')
    expect(screen.getByTestId('task-id')).toHaveTextContent('task-2')
    expect(screen.getByTestId('panel-left')).toHaveTextContent('60')

    // Switch back to task-1 (should preserve previous state)
    act(() => {
      screen.getByTestId('switch-to-task-1').click()
    })

    expect(screen.getByTestId('active-task-id')).toHaveTextContent('task-1')
    expect(screen.getByTestId('task-id')).toHaveTextContent('task-1')
  })
})
