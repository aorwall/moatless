import { useNavigate } from '@tanstack/react-router'
import { useMemo } from 'react'
import { useIsMobile } from '@/lib/hooks/use-mobile'
import { Route } from '@/routes/tasks/$task-id'
import type { TaskView } from '../contexts/task-ui-provider'

interface UseTaskViewOptions {
  serverManagement?: boolean
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
 * - Desktop: 'chat' is always visible on left, URL controls right panel (files/preview)
 * - Mobile: URL controls which single panel is visible (chat/files/preview)
 *
 * Default view logic when no URL param:
 * - Desktop with serverManagement: 'preview'
 * - Desktop without serverManagement: 'files'
 * - Mobile: 'chat'
 */
export function useTaskView({ serverManagement }: UseTaskViewOptions = {}): UseTaskViewReturn {
  const navigate = useNavigate()
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

    // Desktop default: preview if serverManagement, otherwise files
    return serverManagement ? 'preview' : 'files'
  }, [search.view, isMobile, serverManagement])

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
  const navigate = useNavigate()
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
