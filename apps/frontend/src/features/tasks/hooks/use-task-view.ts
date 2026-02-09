import type { ServerEntry } from '@moatless/api-client'
import { useMemo } from 'react'
import { useIsMobile } from '@/lib/hooks/use-mobile'
import { Route } from '@/routes/tasks/$task-id'

// Type exports for task view management
// TaskView is a string: 'chat', 'diff', or any server name (e.g. 'files', 'preview', 'storybook')
export type TaskView = string
export type ViewportMode = 'mobile' | 'tablet' | 'desktop'

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
  const navigate = Route.useNavigate()
  const search = Route.useSearch()
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
      return defaultServer.name
    }

    // Fallback (servers not loaded yet)
    return 'files'
  }, [search.view, isMobile, servers])

  // Function to update the view in URL
  const setView = (newView: TaskView) => {
    navigate({
      search: (prev) => ({ ...prev, view: newView }),
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
  const navigate = Route.useNavigate()
  const search = Route.useSearch()

  const selectedFile = search.file

  // Function to update the selected file in URL
  const setSelectedFile = (file: string | undefined) => {
    navigate({
      search: (prev) => {
        // Remove file param if undefined, otherwise set it
        if (file === undefined) {
          const { file: _, ...rest } = prev
          return rest
        }
        return { ...prev, file }
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
  const navigate = Route.useNavigate()
  const search = Route.useSearch()

  const previewPath = search.path ?? '/'

  const setPreviewPath = (path: string) => {
    navigate({
      search: (prev) => ({ ...prev, path }),
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
  const navigate = Route.useNavigate()
  const search = Route.useSearch()

  const viewport = search.viewport ?? 'desktop'

  const setViewport = (mode: ViewportMode) => {
    navigate({
      search: (prev) => ({ ...prev, viewport: mode }),
      replace: true, // Use replace to avoid polluting browser history
    })
  }

  return { viewport, setViewport }
}
