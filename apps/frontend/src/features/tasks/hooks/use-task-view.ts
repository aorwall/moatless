import type { ServerEntry } from '@moatless/api-client'
import { useNavigate, useSearch } from '@tanstack/react-router'
import type { TaskSearch } from '../types/task-search-schema'

// Type exports for task view management
// TaskView is a string: 'chat', 'diff', or any server name (e.g. 'files', 'preview', 'storybook')
export type TaskView = string

/**
 * Read the task search params from whichever task route is currently active
 * (`/tasks/$task-id/` or `/tasks/$task-id/kiosk`). Both routes share the
 * same `taskSearchSchema`, so casting the loose result is safe.
 */
function useTaskSearch(): TaskSearch {
  return useSearch({ strict: false }) as TaskSearch
}

interface UseTaskViewOptions {
  servers?: ServerEntry[] | undefined
  /**
   * View to use when the URL has no `?view=` param. Layouts supply this so the
   * responsive default is a layout decision, not something the hook derives from
   * a media query (the mobile layout passes `'chat'`). When omitted, the default
   * is derived from the servers list (or `'files'`).
   */
  fallback?: TaskView | undefined
}

interface UseTaskViewReturn {
  view: TaskView
  setView: (view: TaskView) => void
}

/**
 * Hook for managing task view state via URL search params.
 *
 * View behavior:
 * - Desktop: 'chat' is always visible on left, URL controls right panel (server name or 'diff')
 * - Mobile: URL controls which single panel is visible (chat or a server name)
 *
 * Default view logic when no URL param:
 * - `fallback` when the layout provides one (mobile passes `'chat'`)
 * - otherwise the default server name from the servers list, or `'files'`
 *
 * Note: The task page component keeps showing a skeleton while workspace status loads.
 */
export function useTaskView({ servers, fallback }: UseTaskViewOptions = {}): UseTaskViewReturn {
  const navigate = useNavigate()
  const search = useTaskSearch()

  // Compute the effective view
  const view = (() => {
    // If view is explicitly set in URL, use it
    if (search.view) {
      return search.view
    }

    // Layout-provided default (e.g. the mobile layout defaults to chat)
    if (fallback) {
      return fallback
    }

    // Use default server name from the servers array when available
    if (servers && servers.length > 0) {
      const defaultServer = servers.find((s) => s.default) ?? servers[0]
      if (defaultServer === undefined) {
        return 'files'
      }
      // Show files while server is still starting
      if (defaultServer.status === 'started') {
        return defaultServer.name
      }
      return 'files'
    }

    // Fallback (servers not loaded yet)
    return 'files'
  })()

  // Function to update the view in URL
  const setView = (newView: TaskView) => {
    navigate({
      to: '.',
      search: (prev) => {
        if (newView === 'files') {
          const { path: _path, device: _device, ...rest } = prev as TaskSearch
          return { ...rest, view: newView }
        }

        return { ...prev, view: newView } as TaskSearch
      },
      replace: true, // Use replace to avoid polluting browser history
    }).catch(() => {
      /* ignore: navigate() for URL param updates only rejects on framework errors */
    })
  }

  return { view, setView }
}
