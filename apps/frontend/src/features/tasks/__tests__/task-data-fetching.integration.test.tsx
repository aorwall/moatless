import { render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { tasksApi } from '@/features/tasks/api/tasks'
import { createMockTask, createMockTaskEvent } from '@/test-utils/mocks'
import { createWrapper } from '@/test-utils/render'

// Mock the router
const mockNavigate = vi.fn()
const mockParams = { taskId: 'test-task-123' }

vi.mock('@tanstack/react-router', async (importOriginal) => {
  const actual = (await importOriginal()) as typeof import('@tanstack/react-router')
  return {
    ...actual,
    useNavigate: () => mockNavigate,
    useParams: () => mockParams,
    Link: ({ children }: { children: React.ReactNode }) => <a href="/">{children}</a>,
    createFileRoute: (_path: string) => (config: Record<string, unknown>) => ({
      ...config,
      useParams: () => mockParams,
    }),
  }
})

// Mock APIs
vi.mock('@/features/tasks/api/tasks', () => ({
  tasksApi: {
    getTask: vi.fn(),
    getTaskEvents: vi.fn(),
    getTaskGitDiff: vi.fn(),
    getTaskWorkspaceStatus: vi.fn(),
  },
}))

// Mock SSE connection
vi.mock('@/features/tasks/hooks/use-task-sse-connection', () => ({
  useTaskSSEConnection: vi.fn(() => ({
    events: [],
    isConnected: true,
    error: null,
  })),
}))

// Import the route directly
import { Route } from '@/routes/tasks/$taskId'

const TaskDetail = Route.component

describe('Task Data Fetching Integration', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('should fetch task data only once when opening a running task', async () => {
    // Tracking object for API calls
    const apiCallTracker = {
      getTask: 0,
      getTaskEvents: 0,
      getGitDiff: 0,
      getTaskWorkspaceStatus: 0,
    }

    // Setup mock data
    const mockTask = createMockTask({
      taskId: 'test-task-123',
      status: 'running',
      title: 'Test running task',
    })

    const mockEvents = [
      createMockTaskEvent({ taskId: 'test-task-123', type: 'message' }),
      createMockTaskEvent({ taskId: 'test-task-123', type: 'tool_use' }),
    ]

    const mockGitDiff = {
      diff: 'diff --git a/file.ts b/file.ts\n+added line',
      files: [{ path: 'file.ts', additions: 1, deletions: 0 }],
    }

    const mockWorkspaceStatus = {
      status: 'idle',
      hasUncommittedChanges: true,
      hasUnmergedCommits: false,
      isBehindBase: false,
    }

    // Setup API mocks with tracking
    vi.mocked(tasksApi.getTask).mockImplementation(async () => {
      apiCallTracker.getTask++
      console.log(`getTask called: ${apiCallTracker.getTask} times`)
      return mockTask
    })

    vi.mocked(tasksApi.getTaskEvents).mockImplementation(async () => {
      apiCallTracker.getTaskEvents++
      console.log(`getTaskEvents called: ${apiCallTracker.getTaskEvents} times`)
      return { events: mockEvents, items: mockEvents }
    })

    vi.mocked(tasksApi.getGitDiff).mockImplementation(async () => {
      apiCallTracker.getGitDiff++
      console.log(`getGitDiff called: ${apiCallTracker.getGitDiff} times`)
      return mockGitDiff
    })

    vi.mocked(tasksApi.getTaskWorkspaceStatus).mockImplementation(async () => {
      apiCallTracker.getTaskWorkspaceStatus++
      console.log(`getTaskWorkspaceStatus called: ${apiCallTracker.getTaskWorkspaceStatus} times`)
      return mockWorkspaceStatus
    })

    // Render the component
    render(<TaskDetail />, {
      wrapper: createWrapper(),
    })

    // Wait for initial render
    await waitFor(
      () => {
        expect(screen.getByText('Test running task')).toBeInTheDocument()
      },
      { timeout: 5000 },
    )

    // Give some time for any additional renders/fetches
    await new Promise((resolve) => setTimeout(resolve, 1000))

    // Log final counts
    console.log('Final API call counts:', apiCallTracker)

    // Assertions - each API should be called only once
    expect(apiCallTracker.getTask).toBe(1)
    expect(apiCallTracker.getTaskEvents).toBe(1)
    expect(apiCallTracker.getGitDiff).toBe(1)
    expect(apiCallTracker.getTaskWorkspaceStatus).toBe(1)

    // Total API calls should be 4 (one of each)
    const totalCalls = Object.values(apiCallTracker).reduce((sum, count) => sum + count, 0)
    expect(totalCalls).toBe(4)
  })

  it('should show the issue: currently fetches data multiple times', async () => {
    // This test will demonstrate the current behavior
    const apiCallTracker = {
      getTask: 0,
      getTaskEvents: 0,
      getGitDiff: 0,
      getTaskWorkspaceStatus: 0,
    }

    // Setup same mocks
    const mockTask = createMockTask({
      taskId: 'test-task-123',
      status: 'running',
      title: 'Test running task',
    })

    vi.mocked(tasksApi.getTask).mockImplementation(async () => {
      apiCallTracker.getTask++
      return mockTask
    })

    vi.mocked(tasksApi.getTaskEvents).mockImplementation(async () => {
      apiCallTracker.getTaskEvents++
      return { events: [], items: [] }
    })

    vi.mocked(tasksApi.getGitDiff).mockImplementation(async () => {
      apiCallTracker.getGitDiff++
      return { diff: '', files: [] }
    })

    vi.mocked(tasksApi.getTaskWorkspaceStatus).mockImplementation(async () => {
      apiCallTracker.getTaskWorkspaceStatus++
      return {
        status: 'idle',
        hasUncommittedChanges: false,
        hasUnmergedCommits: false,
        isBehindBase: false,
      }
    })

    // Render the component
    render(<TaskDetail />, {
      wrapper: createWrapper(),
    })

    // Wait for component to settle
    await waitFor(
      () => {
        expect(screen.getByText('Test running task')).toBeInTheDocument()
      },
      { timeout: 5000 },
    )

    // Give time for all renders/fetches
    await new Promise((resolve) => setTimeout(resolve, 2000))

    // Log the actual behavior
    console.log('Current behavior - API call counts:', apiCallTracker)

    // This will show the problem - APIs are called multiple times
    console.log(
      `Total API calls: ${Object.values(apiCallTracker).reduce((sum, count) => sum + count, 0)}`,
    )

    // We expect this to fail, showing the issue
    const totalCalls = Object.values(apiCallTracker).reduce((sum, count) => sum + count, 0)
    expect(totalCalls).toBeGreaterThan(4) // This demonstrates the problem
  })
})
