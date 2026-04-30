import type { ServerEntry } from '@moatless/api-client'
import { useNavigate, useSearch } from '@tanstack/react-router'
import { useMemo } from 'react'
import { useIsMobile } from '@/lib/hooks/use-mobile'
import type { TaskSearch } from '../types/task-search-schema'

// Type exports for task view management
// TaskView is a string: 'chat', 'diff', or any server name (e.g. 'files', 'preview', 'storybook')
export type TaskView = string
export type ViewportMode = 'mobile' | 'tablet' | 'desktop'

/**
 * Read the task search params from whichever task route is currently active
 * (`/tasks/$task-id/` or `/tasks/$task-id/headless`). Both routes share the
 * same `taskSearchSchema`, so casting the loose result is safe.
 */
function useTaskSearch(): TaskSearch {
  return useSearch({ strict: false }) as TaskSearch
}

interface UseTaskViewOptions {
  servers?: ServerEntry[]
}

interface UseTaskViewReturn {
  view: TaskView
  setView: (view: TaskView) => void
  isMobile: boolean
}

/**
 * Hook for managing task view state via URL search params.
 *
 * View behavior:
 * - Desktop: 'chat' is always visible on left, URL controls right panel (server name or 'diff')
 * - Mobile: URL controls which single panel is visible (chat or a server name)
 *
 * Default view logic when no URL param:
 * - Mobile: 'chat'
 * - Desktop: default server name from the servers list, or fallback to 'preview'/'files'
 *
 * Note: The task page component keeps showing a skeleton while workspace status loads.
 */
export function useTaskView({ servers }: UseTaskViewOptions = {}): UseTaskViewReturn {
  const navigate = useNavigate()
  const search = useTaskSearch()
  const isMobile = useIsMobile()

  // Compute the effective view
  const view = useMemo<TaskView>(() => {
    // If view is explicitly set in URL, use it
    if (search.view) {
      return search.view
    }

    // Mobile default: chat
    if (isMobile) {
      return 'chat'
    }

    // Use default server name from the servers array when available
    if (servers && servers.length > 0) {
      const defaultServer = servers.find((s) => s.default) ?? servers[0]
      // Show files while server is still starting
      if (defaultServer.status === 'started') {
        return defaultServer.name
      }
      return 'files'
    }

    // Fallback (servers not loaded yet)
    return 'files'
  }, [search.view, isMobile, servers])

  // Function to update the view in URL
  const setView = (newView: TaskView) => {
    navigate({
      to: '.',
      search: (prev) => ({ ...prev, view: newView }) as TaskSearch,
      replace: true, // Use replace to avoid polluting browser history
    })
  }

  return { view, setView, isMobile }
}

interface UseSelectedFileReturn {
  selectedFile: string | undefined
  setSelectedFile: (file: string | undefined) => void
}

/**
 * Hook for managing selected file state via URL search params.
 *
 * The selected file path is stored in the URL, making it:
 * - Shareable via URL
 * - Persistent across page refreshes
 * - Compatible with browser back/forward navigation
 */
export function useSelectedFile(): UseSelectedFileReturn {
  const navigate = useNavigate()
  const search = useTaskSearch()

  const selectedFile = search.file

  // Function to update the selected file in URL
  const setSelectedFile = (file: string | undefined) => {
    navigate({
      to: '.',
      search: (prev) => {
        const prevSearch = prev as TaskSearch
        // Remove file param if undefined, otherwise set it
        if (file === undefined) {
          const { file: _, ...rest } = prevSearch
          return rest
        }
        return { ...prevSearch, file }
      },
      replace: true, // Use replace to avoid polluting browser history
    })
  }

  return { selectedFile, setSelectedFile }
}

interface UsePreviewPathReturn {
  previewPath: string
  setPreviewPath: (path: string) => void
}

/**
 * Hook for managing preview path state via URL search params.
 *
 * The preview path is stored in the URL, making it:
 * - Shareable via URL
 * - Persistent across page refreshes
 * - Compatible with browser back/forward navigation
 */
export function usePreviewPath(): UsePreviewPathReturn {
  const navigate = useNavigate()
  const search = useTaskSearch()

  const previewPath = search.path ?? '/'

  const setPreviewPath = (path: string) => {
    navigate({
      to: '.',
      search: (prev) => ({ ...prev, path }) as TaskSearch,
      replace: true, // Use replace to avoid polluting browser history
    })
  }

  return { previewPath, setPreviewPath }
}

interface UsePreviewViewportReturn {
  viewport: ViewportMode
  setViewport: (mode: ViewportMode) => void
}

/**
 * Hook for managing preview viewport state via URL search params.
 *
 * The viewport mode is stored in the URL, making it:
 * - Shareable via URL (useful for bug reports at specific viewport sizes)
 * - Persistent across page refreshes
 * - Compatible with browser back/forward navigation
 */
export function usePreviewViewport(): UsePreviewViewportReturn {
  const navigate = useNavigate()
  const search = useTaskSearch()

  const viewport = search.viewport ?? 'desktop'

  const setViewport = (mode: ViewportMode) => {
    navigate({
      to: '.',
      search: (prev) => ({ ...prev, viewport: mode }) as TaskSearch,
      replace: true, // Use replace to avoid polluting browser history
    })
  }

  return { viewport, setViewport }
}
