import { act, render, screen } from '@testing-library/react'
import React from 'react'
import { describe, expect, it, vi } from 'vitest'
import { TaskUIProvider, useGitDiffState } from '@/features/tasks/contexts/task-ui-provider'

// Mock scroll functionality
const mockScrollIntoView = vi.fn()
Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
  configurable: true,
  value: mockScrollIntoView,
})

// Test component that simulates git diff viewer behavior
function GitDiffTestComponent() {
  const { gitDiffState, updateGitDiffState } = useGitDiffState()

  const handleExpandFile = (fileKey: string) => {
    const newExpanded = new Set(gitDiffState.expandedFiles)
    if (newExpanded.has(fileKey)) {
      newExpanded.delete(fileKey)
    } else {
      newExpanded.add(fileKey)
    }
    updateGitDiffState({ expandedFiles: newExpanded })
  }

  const handleSelectFile = (fileKey: string) => {
    updateGitDiffState({ selectedFileKey: fileKey })
  }

  const handleExpandPath = (path: string) => {
    const newExpanded = new Set(gitDiffState.expandedPaths)
    if (newExpanded.has(path)) {
      newExpanded.delete(path)
    } else {
      newExpanded.add(path)
    }
    updateGitDiffState({ expandedPaths: newExpanded })
  }

  const handleBulkExpandFiles = (fileKeys: string[]) => {
    updateGitDiffState({ expandedFiles: new Set(fileKeys) })
  }

  const handleResetState = () => {
    updateGitDiffState({
      expandedFiles: new Set(),
      selectedFileKey: undefined,
      expandedPaths: new Set(),
    })
  }

  return (
    <div>
      <div data-testid="expanded-files-count">{gitDiffState.expandedFiles.size}</div>
      <div data-testid="expanded-files-list">
        {Array.from(gitDiffState.expandedFiles).join(',')}
      </div>
      <div data-testid="selected-file-key">{gitDiffState.selectedFileKey || 'none'}</div>
      <div data-testid="expanded-paths-count">{gitDiffState.expandedPaths.size}</div>
      <div data-testid="expanded-paths-list">
        {Array.from(gitDiffState.expandedPaths).join(',')}
      </div>

      <button data-testid="expand-file-1" onClick={() => handleExpandFile('src/file1.ts')}>
        Toggle File 1
      </button>

      <button data-testid="expand-file-2" onClick={() => handleExpandFile('src/file2.ts')}>
        Toggle File 2
      </button>

      <button data-testid="expand-file-3" onClick={() => handleExpandFile('tests/file3.test.ts')}>
        Toggle File 3
      </button>

      <button data-testid="select-file-1" onClick={() => handleSelectFile('src/file1.ts')}>
        Select File 1
      </button>

      <button data-testid="select-file-2" onClick={() => handleSelectFile('src/file2.ts')}>
        Select File 2
      </button>

      <button data-testid="expand-path-src" onClick={() => handleExpandPath('src/')}>
        Toggle Src Path
      </button>

      <button data-testid="expand-path-tests" onClick={() => handleExpandPath('tests/')}>
        Toggle Tests Path
      </button>

      <button
        data-testid="expand-path-components"
        onClick={() => handleExpandPath('src/components/')}
      >
        Toggle Components Path
      </button>

      <button
        data-testid="bulk-expand-files"
        onClick={() => handleBulkExpandFiles(['file1.ts', 'file2.ts', 'file3.ts', 'file4.ts'])}
      >
        Bulk Expand Files
      </button>

      <button data-testid="reset-state" onClick={handleResetState}>
        Reset State
      </button>
    </div>
  )
}

// Multi-task component for testing git diff state isolation
function MultiTaskGitDiffTest() {
  const [currentTaskId, setCurrentTaskId] = React.useState('git-task-1')

  return (
    <div>
      <div data-testid="current-task-id">{currentTaskId}</div>

      <button data-testid="switch-to-git-task-1" onClick={() => setCurrentTaskId('git-task-1')}>
        Switch to Git Task 1
      </button>

      <button data-testid="switch-to-git-task-2" onClick={() => setCurrentTaskId('git-task-2')}>
        Switch to Git Task 2
      </button>

      <button data-testid="switch-to-git-task-3" onClick={() => setCurrentTaskId('git-task-3')}>
        Switch to Git Task 3
      </button>

      <TaskUIProvider taskId={currentTaskId}>
        <GitDiffTestComponent />
      </TaskUIProvider>
    </div>
  )
}

describe('Git Diff State Isolation', () => {
  it('should maintain default git diff state for new tasks', () => {
    render(
      <TaskUIProvider taskId="new-git-task">
        <GitDiffTestComponent />
      </TaskUIProvider>,
    )

    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('0')
    expect(screen.getByTestId('expanded-files-list')).toHaveTextContent('')
    expect(screen.getByTestId('selected-file-key')).toHaveTextContent('none')
    expect(screen.getByTestId('expanded-paths-count')).toHaveTextContent('0')
    expect(screen.getByTestId('expanded-paths-list')).toHaveTextContent('')
  })

  it('should update git diff state within a single task', () => {
    render(
      <TaskUIProvider taskId="single-git-task">
        <GitDiffTestComponent />
      </TaskUIProvider>,
    )

    // Expand some files one by one
    act(() => {
      screen.getByTestId('expand-file-1').click()
    })

    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('1')
    expect(screen.getByTestId('expanded-files-list')).toContain('src/file1.ts')

    act(() => {
      screen.getByTestId('expand-file-2').click()
    })

    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('2')
    expect(screen.getByTestId('expanded-files-list')).toContain('src/file1.ts')
    expect(screen.getByTestId('expanded-files-list')).toContain('src/file2.ts')

    // Select a file
    act(() => {
      screen.getByTestId('select-file-1').click()
    })

    expect(screen.getByTestId('selected-file-key')).toHaveTextContent('src/file1.ts')

    // Expand some paths
    act(() => {
      screen.getByTestId('expand-path-src').click()
      screen.getByTestId('expand-path-tests').click()
    })

    expect(screen.getByTestId('expanded-paths-count')).toHaveTextContent('2')
    expect(screen.getByTestId('expanded-paths-list')).toContain('src/')
    expect(screen.getByTestId('expanded-paths-list')).toContain('tests/')
  })

  it('should isolate git diff state between different tasks', () => {
    render(<MultiTaskGitDiffTest />)

    // Task 1: Expand some files and select one
    expect(screen.getByTestId('current-task-id')).toHaveTextContent('git-task-1')

    act(() => {
      screen.getByTestId('expand-file-1').click()
    })

    act(() => {
      screen.getByTestId('expand-file-2').click()
    })

    act(() => {
      screen.getByTestId('select-file-1').click()
    })

    act(() => {
      screen.getByTestId('expand-path-src').click()
    })

    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('2')
    expect(screen.getByTestId('selected-file-key')).toHaveTextContent('src/file1.ts')
    expect(screen.getByTestId('expanded-paths-count')).toHaveTextContent('1')

    // Switch to Task 2 (should have default state)
    act(() => {
      screen.getByTestId('switch-to-git-task-2').click()
    })

    expect(screen.getByTestId('current-task-id')).toHaveTextContent('git-task-2')
    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('0') // default
    expect(screen.getByTestId('selected-file-key')).toHaveTextContent('none') // default
    expect(screen.getByTestId('expanded-paths-count')).toHaveTextContent('0') // default

    // Task 2: Different state
    act(() => {
      screen.getByTestId('expand-file-3').click()
      screen.getByTestId('select-file-2').click()
      screen.getByTestId('expand-path-tests').click()
      screen.getByTestId('expand-path-components').click()
    })

    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('1')
    expect(screen.getByTestId('selected-file-key')).toHaveTextContent('src/file2.ts')
    expect(screen.getByTestId('expanded-paths-count')).toHaveTextContent('2')

    // Switch back to Task 1 (should restore previous state)
    act(() => {
      screen.getByTestId('switch-to-git-task-1').click()
    })

    expect(screen.getByTestId('current-task-id')).toHaveTextContent('git-task-1')
    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('2') // preserved
    expect(screen.getByTestId('selected-file-key')).toHaveTextContent('src/file1.ts') // preserved
    expect(screen.getByTestId('expanded-paths-count')).toHaveTextContent('1') // preserved

    // Verify specific files and paths are preserved
    expect(screen.getByTestId('expanded-files-list')).toContain('src/file1.ts')
    expect(screen.getByTestId('expanded-files-list')).toContain('src/file2.ts')
    expect(screen.getByTestId('expanded-paths-list')).toContain('src/')

    // Switch back to Task 2 and verify its state is preserved
    act(() => {
      screen.getByTestId('switch-to-git-task-2').click()
    })

    expect(screen.getByTestId('current-task-id')).toHaveTextContent('git-task-2')
    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('1') // preserved
    expect(screen.getByTestId('selected-file-key')).toHaveTextContent('src/file2.ts') // preserved
    expect(screen.getByTestId('expanded-paths-count')).toHaveTextContent('2') // preserved
    expect(screen.getByTestId('expanded-files-list')).toContain('tests/file3.test.ts')
    expect(screen.getByTestId('expanded-paths-list')).toContain('tests/')
    expect(screen.getByTestId('expanded-paths-list')).toContain('src/components/')
  })

  it('should handle bulk operations and state resets independently per task', () => {
    render(<MultiTaskGitDiffTest />)

    // Task 1: Bulk expand
    act(() => {
      screen.getByTestId('bulk-expand-files').click()
    })

    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('4')

    // Switch to Task 2
    act(() => {
      screen.getByTestId('switch-to-git-task-2').click()
    })

    // Task 2: Should have default state (not affected by Task 1's bulk operation)
    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('0')

    // Task 2: Do some manual expansions
    act(() => {
      screen.getByTestId('expand-file-1').click()
      screen.getByTestId('expand-file-3').click()
    })

    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('2')

    // Switch to Task 3
    act(() => {
      screen.getByTestId('switch-to-git-task-3').click()
    })

    // Task 3: Default state
    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('0')

    // Task 3: Bulk expand then reset
    act(() => {
      screen.getByTestId('bulk-expand-files').click()
    })
    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('4')

    act(() => {
      screen.getByTestId('reset-state').click()
    })
    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('0')
    expect(screen.getByTestId('selected-file-key')).toHaveTextContent('none')

    // Verify Task 1 still has its bulk expansion
    act(() => {
      screen.getByTestId('switch-to-git-task-1').click()
    })
    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('4')

    // Verify Task 2 still has its manual expansions
    act(() => {
      screen.getByTestId('switch-to-git-task-2').click()
    })
    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('2')

    // Verify Task 3 still has its reset state
    act(() => {
      screen.getByTestId('switch-to-git-task-3').click()
    })
    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('0')
  })

  it('should handle file expansion toggle operations correctly per task', () => {
    render(<MultiTaskGitDiffTest />)

    // Task 1: Toggle file 1 multiple times
    act(() => {
      screen.getByTestId('expand-file-1').click() // expand
    })
    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('1')

    act(() => {
      screen.getByTestId('expand-file-1').click() // collapse
    })
    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('0')

    act(() => {
      screen.getByTestId('expand-file-1').click() // expand again
    })
    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('1')

    // Switch to Task 2
    act(() => {
      screen.getByTestId('switch-to-git-task-2').click()
    })

    // Task 2: Different toggle pattern
    act(() => {
      screen.getByTestId('expand-file-2').click() // expand file 2
      screen.getByTestId('expand-file-3').click() // expand file 3
    })
    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('2')

    act(() => {
      screen.getByTestId('expand-file-2').click() // collapse file 2
    })
    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('1')
    expect(screen.getByTestId('expanded-files-list')).toContain('tests/file3.test.ts')
    expect(screen.getByTestId('expanded-files-list')).not.toContain('src/file2.ts')

    // Switch back to Task 1 and verify its state
    act(() => {
      screen.getByTestId('switch-to-git-task-1').click()
    })
    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('1')
    expect(screen.getByTestId('expanded-files-list')).toContain('src/file1.ts')

    // Switch back to Task 2 and verify its state
    act(() => {
      screen.getByTestId('switch-to-git-task-2').click()
    })
    expect(screen.getByTestId('expanded-files-count')).toHaveTextContent('1')
    expect(screen.getByTestId('expanded-files-list')).toContain('tests/file3.test.ts')
  })

  it('should handle Set operations correctly without mutation issues', () => {
    render(<MultiTaskGitDiffTest />)

    // Task 1: Build up complex state
    act(() => {
      screen.getByTestId('expand-file-1').click()
      screen.getByTestId('expand-file-2').click()
      screen.getByTestId('expand-path-src').click()
      screen.getByTestId('expand-path-components').click()
    })

    const task1InitialFiles = screen.getByTestId('expanded-files-list').textContent
    const task1InitialPaths = screen.getByTestId('expanded-paths-list').textContent

    // Switch to Task 2 and modify similar state
    act(() => {
      screen.getByTestId('switch-to-git-task-2').click()
    })

    act(() => {
      screen.getByTestId('expand-file-1').click() // Same file as Task 1
      screen.getByTestId('expand-file-3').click() // Different file
      screen.getByTestId('expand-path-src').click() // Same path as Task 1
      screen.getByTestId('expand-path-tests').click() // Different path
    })

    const task2Files = screen.getByTestId('expanded-files-list').textContent
    const task2Paths = screen.getByTestId('expanded-paths-list').textContent

    // Verify Task 2 has its own state
    expect(task2Files).toContain('src/file1.ts')
    expect(task2Files).toContain('tests/file3.test.ts')
    expect(task2Files).not.toContain('src/file2.ts') // Task 1 specific
    expect(task2Paths).toContain('src/')
    expect(task2Paths).toContain('tests/')
    expect(task2Paths).not.toContain('src/components/') // Task 1 specific

    // Switch back to Task 1 and verify no mutation occurred
    act(() => {
      screen.getByTestId('switch-to-git-task-1').click()
    })

    expect(screen.getByTestId('expanded-files-list').textContent).toBe(task1InitialFiles)
    expect(screen.getByTestId('expanded-paths-list').textContent).toBe(task1InitialPaths)

    // Verify Task 1 still has its specific state
    expect(screen.getByTestId('expanded-files-list')).toContain('src/file2.ts')
    expect(screen.getByTestId('expanded-files-list')).not.toContain('tests/file3.test.ts')
    expect(screen.getByTestId('expanded-paths-list')).toContain('src/components/')
    expect(screen.getByTestId('expanded-paths-list')).not.toContain('tests/')
  })
})
