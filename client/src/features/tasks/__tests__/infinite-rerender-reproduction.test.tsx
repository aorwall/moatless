import { render } from '@testing-library/react'
import React from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

// Mock the APIs to return stable data
vi.mock('../api/tasks', () => ({
  tasksApi: {
    getTask: vi.fn().mockResolvedValue({
      taskId: 'test-task-id',
      status: 'running',
      agentType: 'claude-code',
      environmentId: 'test-env',
      iterations: [],
      createdAt: new Date('2024-01-01T00:00:00Z'),
      title: 'Test Task',
      description: 'Test Description',
      updatedAt: new Date('2024-01-01T00:00:00Z'),
      userId: 'test-user',
      lastMessageAt: new Date('2024-01-01T00:00:00Z'),
      numTurns: 0,
      totalCostUsd: 0,
    }),
    getTaskEvents: vi.fn().mockResolvedValue([]),
    getTaskWorkspaceStatus: vi.fn().mockResolvedValue({
      status: 'idle',
      hasUncommittedChanges: false,
      hasUnmergedCommits: false,
      isBehindBase: false,
      gitStatus: {
        hasUncommittedChanges: false,
        hasUnmergedCommits: false,
        isBehindBase: false,
        currentBranch: 'main',
        baseBranch: 'main',
        hasConflicts: false,
        commitsAhead: 0,
        commitsBehind: 0,
      },
    }),
    getTaskGitDiff: vi.fn().mockResolvedValue({ diff: '' }),
  },
}))

// Mock the SSE client to prevent real connections
vi.mock('../api/ndjson-sse', () => ({
  NDJSONSSEClient: vi.fn().mockImplementation(() => ({
    connect: vi.fn(),
    disconnect: vi.fn(),
  })),
}))

// Mock the preview API
vi.mock('@/features/preview', () => ({
  Preview: () => <div>Preview Component</div>,
  useServerStatus: vi.fn().mockReturnValue({
    data: {
      running: false,
      configured: false,
      url: '',
      pid: null,
      uptimeSeconds: 0,
    },
  }),
}))

// Mock file viewer components
vi.mock('@/features/file-viewer', () => ({
  FileTree: () => <div>File Tree</div>,
  FileViewer: () => <div>File Viewer</div>,
}))

// Mock other components that might cause issues
vi.mock('../components/chat-view', () => ({
  ChatView: () => <div>Chat View</div>,
}))

vi.mock('../components/event-viewer', () => ({
  EventViewer: () => <div>Event Viewer</div>,
}))

vi.mock('../components/git-diff-viewer', () => ({
  GitDiffViewer: () => <div>Git Diff Viewer</div>,
}))

describe('Infinite Re-render Pattern Tests', () => {
  let renderCount = 0

  beforeEach(() => {
    renderCount = 0
  })

  it('should detect infinite re-renders caused by new Date() in useMemo', async () => {
    const ProblematicComponent = () => {
      renderCount++
      
      if (renderCount > 50) {
        throw new Error(`INFINITE RE-RENDER DETECTED: Component rendered ${renderCount} times`)
      }

      // This creates a new Date object every render, causing useMemo to always return new object
      const task = React.useMemo(() => {
        return {
          taskId: 'test-task-id',
          createdAt: new Date(), // NEW DATE EVERY TIME!
          iterations: [], // NEW ARRAY EVERY TIME!
        }
      }, []) // Empty deps but object still changes

      // This effect will run on every render because task reference changes
      React.useEffect(() => {
        console.log('Task changed:', task)
      }, [task])

      return <div>Rendered {renderCount} times</div>
    }

    expect(() => {
      render(<ProblematicComponent />)
    }).toThrow(/INFINITE RE-RENDER DETECTED/)
  })

  it('should detect infinite re-renders caused by useEffect with unstable dependency', async () => {
    const ProblematicComponent = () => {
      renderCount++
      
      if (renderCount > 50) {
        throw new Error(`INFINITE RE-RENDER DETECTED: Component rendered ${renderCount} times`)
      }

      const [panelSizes, setPanelSizes] = React.useState({ left: 60, right: 40 })

      React.useEffect(() => {
        // This creates infinite loop because setPanelSizes changes panelSizes
        // which is in the dependency array
        setPanelSizes({ 
          left: 60, 
          right: panelSizes.right 
        })
      }, [panelSizes, setPanelSizes]) // panelSizes changes -> effect runs -> setPanelSizes -> panelSizes changes

      return <div>Panel sizes: {panelSizes.left}% / {panelSizes.right}%</div>
    }

    expect(() => {
      render(<ProblematicComponent />)
    }).toThrow(/INFINITE RE-RENDER DETECTED/)
  })

  it('should detect infinite re-renders caused by useCallback with changing dependencies', async () => {
    const ProblematicComponent = () => {
      renderCount++
      
      if (renderCount > 50) {
        throw new Error(`INFINITE RE-RENDER DETECTED: Component rendered ${renderCount} times`)
      }

      const [refs, setRefs] = React.useState<(HTMLElement | null)[]>([])
      
      // This callback function changes every render because it's not properly memoized
      const setRef = React.useCallback((index: number) => (element: HTMLElement | null) => {
        setRefs(prev => {
          const newRefs = [...prev] // New array every time
          newRefs[index] = element
          return newRefs
        })
      }, []) // Empty deps but function depends on 'index' which is passed at call time

      React.useEffect(() => {
        // This runs every time setRef changes (which is every render)
        console.log('setRef changed')
      }, [setRef])

      return (
        <div>
          {[1, 2, 3].map((item, index) => (
            <div key={item} ref={setRef(index)}>
              Item {item}
            </div>
          ))}
        </div>
      )
    }

    // This test simulates the setRef pattern from the error stack trace
    expect(() => {
      render(<ProblematicComponent />)
    }).toThrow(/INFINITE RE-RENDER DETECTED/)
  })

  it('should show how to fix the patterns correctly', async () => {
    // FIXED VERSION - Stable patterns that don't cause infinite loops
    const FixedComponent = () => {
      renderCount++
      
      // This should not exceed normal render count
      if (renderCount > 5) {
        throw new Error(`Unexpected re-renders: ${renderCount}`)
      }

      // FIXED: Use stable date value
      const stableDate = React.useMemo(() => new Date('2024-01-01T00:00:00Z'), [])
      
      // FIXED: Use stable arrays/objects with proper dependencies
      const task = React.useMemo(() => {
        return {
          taskId: 'test-task-id',
          createdAt: stableDate, // Stable reference
          iterations: [], // Empty array is fine since no deps
        }
      }, [stableDate]) // Proper dependency

      // FIXED: Effect only runs when task actually changes (which it shouldn't)
      React.useEffect(() => {
        console.log('Task changed:', task)
      }, [task])

      // FIXED: Panel sizes only update when needed
      const [panelSizes, setPanelSizes] = React.useState({ left: 60, right: 40 })

      React.useEffect(() => {
        // FIXED: Only run once on mount
        const savedLeft = '65'
        if (savedLeft) {
          setPanelSizes(current => ({ 
            left: Number(savedLeft), 
            right: current.right 
          }))
        }
      }, []) // Empty deps - only run on mount

      return <div>Stable component rendered {renderCount} times</div>
    }

    // This should render without infinite loops
    render(<FixedComponent />)
    
    // Should have rendered a reasonable number of times (initial + maybe one update)
    expect(renderCount).toBeLessThan(5)
  })
}) 