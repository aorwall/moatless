import { act, render, screen, waitFor } from '@testing-library/react'
import React from 'react'
import { describe, expect, it, vi } from 'vitest'
import { TaskUIProvider, useTaskUI } from '@/features/tasks/contexts/task-ui-provider'

// Mock scroll functionality
const mockScrollIntoView = vi.fn()
Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
  configurable: true,
  value: mockScrollIntoView,
})

// Comprehensive test component that uses all UI state features
function ComprehensiveTaskComponent({ taskName }: { taskName: string }) {
  const {
    currentState,
    setRightPanelView,
    setSelectedIteration,
    setPanelSizes,
    setSelectedFile,
    updateChatState,
    updateGitDiffState,
  } = useTaskUI()

  const handleComplexStateUpdate = () => {
    // Simulate a complex workflow with multiple state updates
    updateChatState({
      isAtBottom: false,
      scrollPosition: parseInt(taskName.slice(-1)) * 100,
    })

    updateGitDiffState({
      expandedFiles: new Set([`${taskName}-file1.ts`, `${taskName}-file2.ts`]),
      selectedFileKey: `${taskName}-file1.ts`,
      expandedPaths: new Set([`${taskName}/src/`, `${taskName}/tests/`]),
    })

    setRightPanelView('diff')
    setSelectedFile(`${taskName}/selected.ts`)
    setSelectedIteration({ iterationNumber: parseInt(taskName.slice(-1)) } as any)
    setPanelSizes({
      left: 30 + parseInt(taskName.slice(-1)) * 10,
      right: 70 - parseInt(taskName.slice(-1)) * 10,
    })
  }

  const handleResetState = () => {
    updateChatState({ isAtBottom: true, scrollPosition: 0 })
    updateGitDiffState({
      expandedFiles: new Set(),
      selectedFileKey: undefined,
      expandedPaths: new Set(),
    })
    setRightPanelView('events')
    setSelectedFile('')
    setSelectedIteration(undefined)
    setPanelSizes({ left: 60, right: 40 })
  }

  return (
    <div data-testid={`task-${taskName}`}>
      <div data-testid={`${taskName}-task-name`}>{taskName}</div>

      {/* Chat State */}
      <div data-testid={`${taskName}-chat-is-at-bottom`}>
        {currentState.chatState.isAtBottom.toString()}
      </div>
      <div data-testid={`${taskName}-chat-scroll-position`}>
        {currentState.chatState.scrollPosition || 0}
      </div>

      {/* Git Diff State */}
      <div data-testid={`${taskName}-expanded-files-count`}>
        {currentState.gitDiffState.expandedFiles.size}
      </div>
      <div data-testid={`${taskName}-expanded-files`}>
        {Array.from(currentState.gitDiffState.expandedFiles).join(',')}
      </div>
      <div data-testid={`${taskName}-selected-file-key`}>
        {currentState.gitDiffState.selectedFileKey || 'none'}
      </div>
      <div data-testid={`${taskName}-expanded-paths-count`}>
        {currentState.gitDiffState.expandedPaths.size}
      </div>
      <div data-testid={`${taskName}-expanded-paths`}>
        {Array.from(currentState.gitDiffState.expandedPaths).join(',')}
      </div>

      {/* Panel State */}
      <div data-testid={`${taskName}-panel-view`}>{currentState.rightPanelView}</div>
      <div data-testid={`${taskName}-selected-file`}>{currentState.selectedFile || 'none'}</div>
      <div data-testid={`${taskName}-selected-iteration`}>
        {currentState.selectedIteration?.iterationNumber || 'none'}
      </div>
      <div data-testid={`${taskName}-panel-sizes`}>
        {currentState.panelSizes.left},{currentState.panelSizes.right}
      </div>

      <button
        type="button"
        data-testid={`${taskName}-complex-update`}
        onClick={handleComplexStateUpdate}
      >
        Complex Update
      </button>

      <button type="button" data-testid={`${taskName}-reset-state`} onClick={handleResetState}>
        Reset State
      </button>
    </div>
  )
}

// Multi-task switching test component
function TaskSwitchingTest() {
  const [currentTaskId, setCurrentTaskId] = React.useState('task1')
  const [taskHistory, setTaskHistory] = React.useState<string[]>(['task1'])

  const switchToTask = (taskId: string) => {
    setCurrentTaskId(taskId)
    setTaskHistory((prev) => [...prev, taskId])
  }

  return (
    <div>
      <div data-testid="current-task-id">{currentTaskId}</div>
      <div data-testid="task-history">{taskHistory.join(' -> ')}</div>
      <div data-testid="task-switch-count">{taskHistory.length - 1}</div>

      <button type="button" data-testid="switch-to-task1" onClick={() => switchToTask('task1')}>
        Task 1
      </button>
      <button type="button" data-testid="switch-to-task2" onClick={() => switchToTask('task2')}>
        Task 2
      </button>
      <button type="button" data-testid="switch-to-task3" onClick={() => switchToTask('task3')}>
        Task 3
      </button>
      <button type="button" data-testid="switch-to-task4" onClick={() => switchToTask('task4')}>
        Task 4
      </button>
      <button type="button" data-testid="switch-to-task5" onClick={() => switchToTask('task5')}>
        Task 5
      </button>

      <TaskUIProvider taskId={currentTaskId}>
        <ComprehensiveTaskComponent taskName={currentTaskId} />
      </TaskUIProvider>
    </div>
  )
}

describe('Task Switching Behavior', () => {
  it('should maintain complete state isolation across task switches', async () => {
    render(<TaskSwitchingTest />)

    // Task 1: Set complex state
    act(() => {
      screen.getByTestId('task1-complex-update').click()
    })

    // Capture Task 1 state
    const task1State = {
      chatIsAtBottom: screen.getByTestId('task1-chat-is-at-bottom').textContent,
      chatScrollPosition: screen.getByTestId('task1-chat-scroll-position').textContent,
      expandedFilesCount: screen.getByTestId('task1-expanded-files-count').textContent,
      expandedFiles: screen.getByTestId('task1-expanded-files').textContent,
      selectedFileKey: screen.getByTestId('task1-selected-file-key').textContent,
      expandedPathsCount: screen.getByTestId('task1-expanded-paths-count').textContent,
      expandedPaths: screen.getByTestId('task1-expanded-paths').textContent,
      panelView: screen.getByTestId('task1-panel-view').textContent,
      selectedFile: screen.getByTestId('task1-selected-file').textContent,
      selectedIteration: screen.getByTestId('task1-selected-iteration').textContent,
      panelSizes: screen.getByTestId('task1-panel-sizes').textContent,
    }

    // Switch to Task 2 (should have default state)
    act(() => {
      screen.getByTestId('switch-to-task2').click()
    })

    await waitFor(() => {
      expect(screen.getByTestId('current-task-id')).toHaveTextContent('task2')
      expect(screen.getByTestId('task2-chat-is-at-bottom')).toHaveTextContent('true') // default
      expect(screen.getByTestId('task2-expanded-files-count')).toHaveTextContent('0') // default
      expect(screen.getByTestId('task2-panel-view')).toHaveTextContent('events') // default
    })

    // Task 2: Set different complex state
    act(() => {
      screen.getByTestId('task2-complex-update').click()
    })

    // Capture Task 2 state
    const task2State = {
      chatIsAtBottom: screen.getByTestId('task2-chat-is-at-bottom').textContent,
      chatScrollPosition: screen.getByTestId('task2-chat-scroll-position').textContent,
      expandedFilesCount: screen.getByTestId('task2-expanded-files-count').textContent,
      expandedFiles: screen.getByTestId('task2-expanded-files').textContent,
      selectedFileKey: screen.getByTestId('task2-selected-file-key').textContent,
      panelView: screen.getByTestId('task2-panel-view').textContent,
      selectedIteration: screen.getByTestId('task2-selected-iteration').textContent,
    }

    // Verify Task 2 state is different from Task 1
    expect(task2State.chatScrollPosition).not.toBe(task1State.chatScrollPosition)
    expect(task2State.expandedFiles).not.toBe(task1State.expandedFiles)
    expect(task2State.selectedFileKey).not.toBe(task1State.selectedFileKey)
    expect(task2State.selectedIteration).not.toBe(task1State.selectedIteration)

    // Switch back to Task 1 (should restore exact previous state)
    act(() => {
      screen.getByTestId('switch-to-task1').click()
    })

    await waitFor(() => {
      expect(screen.getByTestId('current-task-id')).toHaveTextContent('task1')
    })
    expect(screen.getByTestId('task1-chat-is-at-bottom').textContent).toBe(
      task1State.chatIsAtBottom,
    )
    expect(screen.getByTestId('task1-chat-scroll-position').textContent).toBe(
      task1State.chatScrollPosition,
    )
    expect(screen.getByTestId('task1-expanded-files-count').textContent).toBe(
      task1State.expandedFilesCount,
    )
    expect(screen.getByTestId('task1-expanded-files').textContent).toBe(task1State.expandedFiles)
    expect(screen.getByTestId('task1-selected-file-key').textContent).toBe(
      task1State.selectedFileKey,
    )
    expect(screen.getByTestId('task1-expanded-paths-count').textContent).toBe(
      task1State.expandedPathsCount,
    )
    expect(screen.getByTestId('task1-expanded-paths').textContent).toBe(task1State.expandedPaths)
    expect(screen.getByTestId('task1-panel-view').textContent).toBe(task1State.panelView)
    expect(screen.getByTestId('task1-selected-file').textContent).toBe(task1State.selectedFile)
    expect(screen.getByTestId('task1-selected-iteration').textContent).toBe(
      task1State.selectedIteration,
    )
    expect(screen.getByTestId('task1-panel-sizes').textContent).toBe(task1State.panelSizes)

    // Switch back to Task 2 (should restore its state)
    act(() => {
      screen.getByTestId('switch-to-task2').click()
    })

    expect(screen.getByTestId('task2-chat-is-at-bottom').textContent).toBe(
      task2State.chatIsAtBottom,
    )
    expect(screen.getByTestId('task2-chat-scroll-position').textContent).toBe(
      task2State.chatScrollPosition,
    )
    expect(screen.getByTestId('task2-expanded-files-count').textContent).toBe(
      task2State.expandedFilesCount,
    )
    expect(screen.getByTestId('task2-panel-view').textContent).toBe(task2State.panelView)
    expect(screen.getByTestId('task2-selected-iteration').textContent).toBe(
      task2State.selectedIteration,
    )
  })

  it('should handle rapid task switching without state corruption', async () => {
    render(<TaskSwitchingTest />)

    // Set up different states for multiple tasks
    const taskStates: Record<string, any> = {}

    for (let taskNum = 1; taskNum <= 5; taskNum++) {
      act(() => {
        screen.getByTestId(`switch-to-task${taskNum}`).click()
      })

      await waitFor(() => {
        expect(screen.getByTestId(`task${taskNum}-complex-update`)).toBeInTheDocument()
      })

      act(() => {
        screen.getByTestId(`task${taskNum}-complex-update`).click()
      })

      taskStates[`task${taskNum}`] = {
        chatScrollPosition: screen.getByTestId(`task${taskNum}-chat-scroll-position`).textContent,
        expandedFiles: screen.getByTestId(`task${taskNum}-expanded-files`).textContent,
        selectedIteration: screen.getByTestId(`task${taskNum}-selected-iteration`).textContent,
      }
    }

    // Rapidly switch between tasks multiple times
    const switchSequence = [
      'task1',
      'task3',
      'task5',
      'task2',
      'task4',
      'task1',
      'task5',
      'task3',
      'task2',
      'task4',
    ]

    for (const taskId of switchSequence) {
      act(() => {
        screen.getByTestId(`switch-to-${taskId}`).click()
      })

      // Verify state is preserved
      await waitFor(() => {
        expect(screen.getByTestId('current-task-id')).toHaveTextContent(taskId)
        expect(screen.getByTestId(`${taskId}-chat-scroll-position`).textContent).toBe(
          taskStates[taskId].chatScrollPosition,
        )
        expect(screen.getByTestId(`${taskId}-expanded-files`).textContent).toBe(
          taskStates[taskId].expandedFiles,
        )
        expect(screen.getByTestId(`${taskId}-selected-iteration`).textContent).toBe(
          taskStates[taskId].selectedIteration,
        )
      })
    }

    // Verify we completed the switching sequence
    expect(screen.getByTestId('task-switch-count')).toHaveTextContent('15') // 5 initial + 10 switches
  })

  it('should handle state modifications during task switching', () => {
    render(<TaskSwitchingTest />)

    // Task 1: Initial state
    act(() => {
      screen.getByTestId('task1-complex-update').click()
    })

    const task1InitialFiles = screen.getByTestId('task1-expanded-files').textContent

    // Switch to Task 2
    act(() => {
      screen.getByTestId('switch-to-task2').click()
      screen.getByTestId('task2-complex-update').click()
    })

    // Switch back to Task 1 and modify state
    act(() => {
      screen.getByTestId('switch-to-task1').click()
    })

    // Verify Task 1 still has its original state
    expect(screen.getByTestId('task1-expanded-files').textContent).toBe(task1InitialFiles)

    // Reset Task 1 state
    act(() => {
      screen.getByTestId('task1-reset-state').click()
    })

    expect(screen.getByTestId('task1-expanded-files-count')).toHaveTextContent('0')
    expect(screen.getByTestId('task1-chat-is-at-bottom')).toHaveTextContent('true')
    expect(screen.getByTestId('task1-panel-view')).toHaveTextContent('events')

    // Switch to Task 2 and verify it wasn't affected by Task 1 reset
    act(() => {
      screen.getByTestId('switch-to-task2').click()
    })

    expect(screen.getByTestId('task2-expanded-files-count')).toHaveTextContent('2') // preserved
    expect(screen.getByTestId('task2-chat-is-at-bottom')).toHaveTextContent('false') // preserved
    expect(screen.getByTestId('task2-panel-view')).toHaveTextContent('diff') // preserved

    // Switch back to Task 1 and verify reset state is preserved
    act(() => {
      screen.getByTestId('switch-to-task1').click()
    })

    expect(screen.getByTestId('task1-expanded-files-count')).toHaveTextContent('0') // reset preserved
    expect(screen.getByTestId('task1-chat-is-at-bottom')).toHaveTextContent('true') // reset preserved
    expect(screen.getByTestId('task1-panel-view')).toHaveTextContent('events') // reset preserved
  })

  it('should maintain state consistency across complex usage patterns', () => {
    render(<TaskSwitchingTest />)

    // Simulate a complex workflow pattern
    const workflow = [
      { task: 'task1', action: 'complex-update' },
      { task: 'task2', action: 'complex-update' },
      { task: 'task1', action: 'verify' }, // verify task1 state preserved
      { task: 'task3', action: 'complex-update' },
      { task: 'task2', action: 'reset-state' },
      { task: 'task1', action: 'verify' }, // verify task1 not affected by task2 reset
      { task: 'task3', action: 'verify' }, // verify task3 not affected by task2 reset
      { task: 'task4', action: 'complex-update' },
      { task: 'task5', action: 'complex-update' },
      { task: 'task2', action: 'verify' }, // verify task2 reset state preserved
    ]

    const expectedStates: Record<string, any> = {}

    for (const step of workflow) {
      act(() => {
        screen.getByTestId(`switch-to-${step.task}`).click()
      })

      if (step.action === 'complex-update') {
        act(() => {
          screen.getByTestId(`${step.task}-complex-update`).click()
        })

        expectedStates[step.task] = {
          chatIsAtBottom: 'false',
          expandedFilesCount: '2',
          panelView: 'diff',
          selectedIteration: step.task.slice(-1), // task number
        }
      } else if (step.action === 'reset-state') {
        act(() => {
          screen.getByTestId(`${step.task}-reset-state`).click()
        })

        expectedStates[step.task] = {
          chatIsAtBottom: 'true',
          expandedFilesCount: '0',
          panelView: 'events',
          selectedIteration: 'none',
        }
      } else if (step.action === 'verify') {
        const expected = expectedStates[step.task]
        if (expected) {
          expect(screen.getByTestId(`${step.task}-chat-is-at-bottom`)).toHaveTextContent(
            expected.chatIsAtBottom,
          )
          expect(screen.getByTestId(`${step.task}-expanded-files-count`)).toHaveTextContent(
            expected.expandedFilesCount,
          )
          expect(screen.getByTestId(`${step.task}-panel-view`)).toHaveTextContent(
            expected.panelView,
          )
          expect(screen.getByTestId(`${step.task}-selected-iteration`)).toHaveTextContent(
            expected.selectedIteration,
          )
        }
      }
    }

    // Final verification: all tasks should have their expected states
    for (const [taskId, expected] of Object.entries(expectedStates)) {
      act(() => {
        screen.getByTestId(`switch-to-${taskId}`).click()
      })

      expect(screen.getByTestId(`${taskId}-chat-is-at-bottom`)).toHaveTextContent(
        expected.chatIsAtBottom,
      )
      expect(screen.getByTestId(`${taskId}-expanded-files-count`)).toHaveTextContent(
        expected.expandedFilesCount,
      )
      expect(screen.getByTestId(`${taskId}-panel-view`)).toHaveTextContent(expected.panelView)
      expect(screen.getByTestId(`${taskId}-selected-iteration`)).toHaveTextContent(
        expected.selectedIteration,
      )
    }
  })

  it('should handle edge cases and boundary conditions', () => {
    render(<TaskSwitchingTest />)

    // Test switching to the same task multiple times
    act(() => {
      screen.getByTestId('task1-complex-update').click()
    })

    const originalState = screen.getByTestId('task1-expanded-files').textContent

    // Switch to same task multiple times
    for (let i = 0; i < 5; i++) {
      act(() => {
        screen.getByTestId('switch-to-task1').click()
      })
      expect(screen.getByTestId('task1-expanded-files').textContent).toBe(originalState)
    }

    // Test immediate switches without state changes
    act(() => {
      screen.getByTestId('switch-to-task2').click()
    })
    act(() => {
      screen.getByTestId('switch-to-task3').click()
    })
    act(() => {
      screen.getByTestId('switch-to-task4').click()
    })
    act(() => {
      screen.getByTestId('switch-to-task5').click()
    })

    // All should have default states
    expect(screen.getByTestId('task5-expanded-files-count')).toHaveTextContent('0')
    expect(screen.getByTestId('task5-chat-is-at-bottom')).toHaveTextContent('true')

    // Switch back to task1 and verify state preserved through all switches
    act(() => {
      screen.getByTestId('switch-to-task1').click()
    })
    expect(screen.getByTestId('task1-expanded-files').textContent).toBe(originalState)
  })
})
