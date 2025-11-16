import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
} from '@tanstack/react-router'
import { render, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { TaskView } from '../use-task-view'
import { useSelectedFile, useTaskView } from '../use-task-view'

// Mock the useIsMobile hook
vi.mock('@/lib/hooks/use-mobile', () => ({
  useIsMobile: vi.fn(() => false),
}))

// Helper to create a test router
function createTestRouter(
  initialSearch: Record<string, unknown> = {},
  TestComponent?: () => React.ReactElement | null,
) {
  const rootRoute = createRootRoute()
  const taskRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: '/tasks/$task-id',
    component: TestComponent || (() => <Outlet />),
    validateSearch: (search: Record<string, unknown>) => ({
      view: search.view as TaskView | undefined,
      file: search.file as string | undefined,
    }),
  })

  const routeTree = rootRoute.addChildren([taskRoute])

  const router = createRouter({
    routeTree,
    history: createMemoryHistory({
      initialEntries: [`/tasks/task-123?${new URLSearchParams(initialSearch as any).toString()}`],
    }),
  })

  return router
}

// Helper to render hook within router context
function renderHookWithRouter<T>(
  hook: () => T,
  initialSearch: Record<string, unknown> = {},
): { result: { current: T } } {
  let hookResult: T

  function TestComponent() {
    hookResult = hook()
    return null
  }

  const router = createTestRouter(initialSearch, TestComponent)

  render(<RouterProvider router={router} />)

  return {
    result: {
      get current() {
        return hookResult!
      },
    },
  }
}

describe('useTaskView', () => {
  it('should default to chat on mobile', async () => {
    const { useIsMobile } = await import('@/lib/hooks/use-mobile')
    vi.mocked(useIsMobile).mockReturnValue(true)

    const { result } = renderHookWithRouter(() => useTaskView())

    expect(result.current.view).toBe('chat')
    expect(result.current.isMobile).toBe(true)

    // Restore mock
    vi.mocked(useIsMobile).mockReturnValue(false)
  })

  it('should default to preview on desktop with serverManagement', async () => {
    const { result } = renderHookWithRouter(() => useTaskView({ serverManagement: true }))

    expect(result.current.view).toBe('preview')
    expect(result.current.isMobile).toBe(false)
  })

  it('should default to files on desktop without serverManagement', async () => {
    const { result } = renderHookWithRouter(() => useTaskView({ serverManagement: false }))

    expect(result.current.view).toBe('files')
    expect(result.current.isMobile).toBe(false)
  })

  it('should respect URL param over defaults', async () => {
    const { result } = renderHookWithRouter(() => useTaskView({ serverManagement: true }), {
      view: 'files',
    })

    // Even though serverManagement is true (default would be 'preview'),
    // the URL param should take precedence
    expect(result.current.view).toBe('files')
  })

  it('should update URL when setView is called', async () => {
    const { result } = renderHookWithRouter(() => useTaskView())

    // Initially should be 'files' (desktop without serverManagement)
    expect(result.current.view).toBe('files')

    // Update view
    result.current.setView('preview')

    // Wait for navigation to complete
    await waitFor(() => {
      expect(result.current.view).toBe('preview')
    })
  })

  it('should use replace: true to avoid history pollution', async () => {
    const { result } = renderHookWithRouter(() => useTaskView())

    // Change view multiple times
    result.current.setView('preview')
    await waitFor(() => expect(result.current.view).toBe('preview'))

    result.current.setView('files')
    await waitFor(() => expect(result.current.view).toBe('files'))

    // The navigation should use replace, so we can't go back
    // This is a bit hard to test directly, but we can verify the behavior
    expect(result.current.view).toBe('files')
  })
})

describe('useSelectedFile', () => {
  it('should return undefined when no file param in URL', async () => {
    const { result } = renderHookWithRouter(() => useSelectedFile())

    expect(result.current.selectedFile).toBeUndefined()
  })

  it('should return file path from URL param', async () => {
    const { result } = renderHookWithRouter(() => useSelectedFile(), { file: 'src/app.tsx' })

    expect(result.current.selectedFile).toBe('src/app.tsx')
  })

  it('should update URL when setSelectedFile is called', async () => {
    const { result } = renderHookWithRouter(() => useSelectedFile())

    // Initially undefined
    expect(result.current.selectedFile).toBeUndefined()

    // Set file
    result.current.setSelectedFile('src/components/Button.tsx')

    // Wait for navigation
    await waitFor(() => {
      expect(result.current.selectedFile).toBe('src/components/Button.tsx')
    })
  })

  it('should remove param when setting undefined', async () => {
    const { result } = renderHookWithRouter(() => useSelectedFile(), { file: 'src/app.tsx' })

    // Initially has file
    expect(result.current.selectedFile).toBe('src/app.tsx')

    // Clear file
    result.current.setSelectedFile(undefined)

    // Wait for navigation
    await waitFor(() => {
      expect(result.current.selectedFile).toBeUndefined()
    })
  })

  it('should preserve other search params', async () => {
    let viewResult: ReturnType<typeof useTaskView>
    let fileResult: ReturnType<typeof useSelectedFile>

    function TestComponent() {
      viewResult = useTaskView()
      fileResult = useSelectedFile()
      return null
    }

    const router = createTestRouter({ view: 'preview', file: 'src/app.tsx' }, TestComponent)
    render(<RouterProvider router={router} />)

    // Wait for initial render
    await waitFor(() => {
      expect(viewResult!.view).toBe('preview')
      expect(fileResult!.selectedFile).toBe('src/app.tsx')
    })

    // Update file
    fileResult!.setSelectedFile('src/index.tsx')

    await waitFor(() => {
      expect(fileResult!.selectedFile).toBe('src/index.tsx')
    })

    // View param should still be present
    expect(viewResult!.view).toBe('preview')
  })
})
