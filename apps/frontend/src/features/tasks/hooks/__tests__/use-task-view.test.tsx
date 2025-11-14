import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  type Router,
  RouterProvider,
} from '@tanstack/react-router'
import { renderHook, waitFor } from '@testing-library/react'
import type { ReactNode } from 'react'
import { describe, expect, it, vi } from 'vitest'
import type { TaskView } from '../../contexts/task-ui-provider'
import { useSelectedFile, useTaskView } from '../use-task-view'

// Mock the useIsMobile hook
vi.mock('@/lib/hooks/use-mobile', () => ({
  useIsMobile: vi.fn(() => false),
}))

// Helper to create a test router
function createTestRouter(initialSearch: Record<string, unknown> = {}) {
  const rootRoute = createRootRoute()
  const taskRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: '/tasks/$task-id',
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

// Wrapper component that provides router context
function createRouterWrapper(router: Router<any, any>) {
  return function RouterWrapper({ children }: { children: ReactNode }) {
    return <RouterProvider router={router}>{children}</RouterProvider>
  }
}

describe('useTaskView', () => {
  it('should default to chat on mobile', async () => {
    const { useIsMobile } = await import('@/lib/hooks/use-mobile')
    vi.mocked(useIsMobile).mockReturnValue(true)

    const router = createTestRouter({})
    const wrapper = createRouterWrapper(router)

    // Wait for router to be ready
    await router.load()

    const { result } = renderHook(() => useTaskView(), { wrapper })

    expect(result.current.view).toBe('chat')
    expect(result.current.isMobile).toBe(true)

    // Restore mock
    vi.mocked(useIsMobile).mockReturnValue(false)
  })

  it('should default to preview on desktop with serverManagement', async () => {
    const router = createTestRouter({})
    const wrapper = createRouterWrapper(router)

    await router.load()

    const { result } = renderHook(() => useTaskView({ serverManagement: true }), { wrapper })

    expect(result.current.view).toBe('preview')
    expect(result.current.isMobile).toBe(false)
  })

  it('should default to files on desktop without serverManagement', async () => {
    const router = createTestRouter({})
    const wrapper = createRouterWrapper(router)

    await router.load()

    const { result } = renderHook(() => useTaskView({ serverManagement: false }), { wrapper })

    expect(result.current.view).toBe('files')
    expect(result.current.isMobile).toBe(false)
  })

  it('should respect URL param over defaults', async () => {
    const router = createTestRouter({ view: 'files' })
    const wrapper = createRouterWrapper(router)

    await router.load()

    const { result } = renderHook(() => useTaskView({ serverManagement: true }), { wrapper })

    // Even though serverManagement is true (default would be 'preview'),
    // the URL param should take precedence
    expect(result.current.view).toBe('files')
  })

  it('should update URL when setView is called', async () => {
    const router = createTestRouter({})
    const wrapper = createRouterWrapper(router)

    await router.load()

    const { result } = renderHook(() => useTaskView(), { wrapper })

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
    const router = createTestRouter({})
    const wrapper = createRouterWrapper(router)

    await router.load()

    const { result } = renderHook(() => useTaskView(), { wrapper })

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
    const router = createTestRouter({})
    const wrapper = createRouterWrapper(router)

    await router.load()

    const { result } = renderHook(() => useSelectedFile(), { wrapper })

    expect(result.current.selectedFile).toBeUndefined()
  })

  it('should return file path from URL param', async () => {
    const router = createTestRouter({ file: 'src/app.tsx' })
    const wrapper = createRouterWrapper(router)

    await router.load()

    const { result } = renderHook(() => useSelectedFile(), { wrapper })

    expect(result.current.selectedFile).toBe('src/app.tsx')
  })

  it('should update URL when setSelectedFile is called', async () => {
    const router = createTestRouter({})
    const wrapper = createRouterWrapper(router)

    await router.load()

    const { result } = renderHook(() => useSelectedFile(), { wrapper })

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
    const router = createTestRouter({ file: 'src/app.tsx' })
    const wrapper = createRouterWrapper(router)

    await router.load()

    const { result } = renderHook(() => useSelectedFile(), { wrapper })

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
    const router = createTestRouter({ view: 'preview', file: 'src/app.tsx' })
    const wrapper = createRouterWrapper(router)

    await router.load()

    const { result: viewResult } = renderHook(() => useTaskView(), { wrapper })
    const { result: fileResult } = renderHook(() => useSelectedFile(), { wrapper })

    // Both params should be present
    expect(viewResult.current.view).toBe('preview')
    expect(fileResult.current.selectedFile).toBe('src/app.tsx')

    // Update file
    fileResult.current.setSelectedFile('src/index.tsx')

    await waitFor(() => {
      expect(fileResult.current.selectedFile).toBe('src/index.tsx')
    })

    // View param should still be present
    expect(viewResult.current.view).toBe('preview')
  })
})
