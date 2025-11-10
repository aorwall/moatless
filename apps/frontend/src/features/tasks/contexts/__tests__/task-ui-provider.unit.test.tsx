import { act, render, screen } from '@testing-library/react'
import React from 'react'
import { describe, expect, it } from 'vitest'
import {
  TaskUIProvider,
  useGitDiffState,
  usePanelView,
  useSelectedIteration,
  useTaskUI,
} from '@/features/tasks'

// Test component that uses all hooks
function TestComponent({ taskId }: { taskId: string }) {
  const { gitDiffState, updateGitDiffState } = useGitDiffState()
  const { rightPanelView, setRightPanelView } = usePanelView()
  const { selectedIteration, setSelectedIteration } = useSelectedIteration()

  return (
    <div>
      <div data-testid="task-id">{taskId}</div>
      <div data-testid="expanded-files-count">{gitDiffState.expandedFiles.size}</div>
      <div data-testid="selected-file-key">{gitDiffState.selectedFileKey || 'none'}</div>
      <div data-testid="expanded-paths-count">{gitDiffState.expandedPaths.size}</div>
      <div data-testid="panel-view">{rightPanelView}</div>
      <div data-testid="selected-iteration">{selectedIteration?.iterationNumber || 'none'}</div>

      <button
        type="button"
        data-testid="update-git-diff-state"
        onClick={() =>
          updateGitDiffState({
            expandedFiles: new Set(['file1.ts', 'file2.ts']),
            selectedFileKey: 'file1.ts',
            expandedPaths: new Set(['src/', 'src/components/']),
          })
        }
      >
        Update Git Diff State
      </button>

      <button
        type="button"
        data-testid="update-panel-view"
        onClick={() => setRightPanelView('files')}
      >
        Update Panel View
      </button>

      <button
        type="button"
        data-testid="update-iteration"
        onClick={() => setSelectedIteration({ iterationNumber: 2 } as any)}
      >
        Update Iteration
      </button>
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

    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('0')
    expect(screen.getByTestId('selected-file-key')).toHaveTextContent('none')
    expect(screen.getByTestId('expanded-paths-count')).toHaveTextContent('0')
    expect(screen.getByTestId('panel-view')).toHaveTextContent('files')
    expect(screen.getByTestId('selected-iteration')).toHaveTextContent('none')
  })

  it('should update git diff state correctly', async () => {
    render(
      <TaskUIProvider taskId="task-1">
        <TestComponent taskId="task-1" />
      </TaskUIProvider>,
    )

    act(() => {
      screen.getByTestId('update-git-diff-state').click()
    })

    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('2')
    expect(screen.getByTestId('selected-file-key')).toHaveTextContent('file1.ts')
    expect(screen.getByTestId('expanded-paths-count')).toHaveTextContent('2')
  })

  it('should update panel view and iteration correctly', async () => {
    render(
      <TaskUIProvider taskId="task-1">
        <TestComponent taskId="task-1" />
      </TaskUIProvider>,
    )

    act(() => {
      screen.getByTestId('update-panel-view').click()
    })

    act(() => {
      screen.getByTestId('update-iteration').click()
    })

    expect(screen.getByTestId('panel-view')).toHaveTextContent('files')
    expect(screen.getByTestId('selected-iteration')).toHaveTextContent('2')
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
    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('0') // default
    expect(screen.getByTestId('panel-view')).toHaveTextContent('files') // default

    // Start with task-1, update its git diff state

    act(() => {
      screen.getByTestId('update-git-diff-state').click()
    })

    // Verify task-1 git diff state is updated
    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('2')

    act(() => {
      screen.getByTestId('update-panel-view').click()
    })

    // Verify task-1 panel state is updated
    expect(screen.getByTestId('panel-view')).toHaveTextContent('files')

    // Switch to task-2 (should have default state)
    act(() => {
      screen.getByTestId('switch-to-task-2').click()
    })

    expect(screen.getByTestId('active-task-id')).toHaveTextContent('task-2')
    expect(screen.getByTestId('task-id')).toHaveTextContent('task-2')
    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('0') // default
    expect(screen.getByTestId('panel-view')).toHaveTextContent('files') // default

    // Update task-2 state differently
    act(() => {
      screen.getByTestId('update-iteration').click()
    })

    expect(screen.getByTestId('selected-iteration')).toHaveTextContent('2')

    // Switch back to task-1 (should preserve previous state)
    act(() => {
      screen.getByTestId('switch-to-task-1').click()
    })

    expect(screen.getByTestId('active-task-id')).toHaveTextContent('task-1')
    expect(screen.getByTestId('task-id')).toHaveTextContent('task-1')
    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('2') // preserved
    expect(screen.getByTestId('panel-view')).toHaveTextContent('files') // preserved
    expect(screen.getByTestId('selected-iteration')).toHaveTextContent('none') // task-1 didn't set this

    // Switch back to task-2 (should preserve its state)
    act(() => {
      screen.getByTestId('switch-to-task-2').click()
    })

    expect(screen.getByTestId('active-task-id')).toHaveTextContent('task-2')
    expect(screen.getByTestId('task-id')).toHaveTextContent('task-2')
    expect(screen.getByTestId('selected-iteration')).toHaveTextContent('2') // task-2 preserved
  })

  it('should maintain independent state updates across tasks', () => {
    render(<MultiTaskTest />)

    // Task 1: Set git diff state
    act(() => {
      screen.getByTestId('update-git-diff-state').click()
    })

    const task1GitState = screen.getByTestId('expanded-files-count').textContent

    // Switch to Task 2
    act(() => {
      screen.getByTestId('switch-to-task-2').click()
    })

    // Task 2: Set different iteration state
    act(() => {
      screen.getByTestId('update-iteration').click()
    })

    const task2IterationState = screen.getByTestId('selected-iteration').textContent

    // Switch back to Task 1
    act(() => {
      screen.getByTestId('switch-to-task-1').click()
    })

    // Task 1 should have its git state preserved but not task 2's iteration state
    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent(task1GitState!)
    expect(screen.getByTestId('selected-iteration')).toHaveTextContent('none') // default, not task 2's state

    // Switch back to Task 2
    act(() => {
      screen.getByTestId('switch-to-task-2').click()
    })

    // Task 2 should have its iteration state but not task 1's git state
    expect(screen.getByTestId('selected-iteration')).toHaveTextContent(task2IterationState!)
    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('0') // default, not task 1's state
  })
})
