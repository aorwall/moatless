import type { MessageContextInput } from '@moatless/api-client'
import {
  createContext,
  type Dispatch,
  type ReactNode,
  type SetStateAction,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from 'react'
import { toast } from 'sonner'
import type { ElementInfo } from '@/features/preview/types'
import { usePreviewPath } from '../hooks/use-task-view'

interface ComposerContextValue {
  contextItems: MessageContextInput[]
  setContextItems: Dispatch<SetStateAction<MessageContextInput[]>>
  /** Toggle an inspected element in the composer context (add if absent, remove if present). */
  toggleElementContext: (element: ElementInfo) => void
}

const ComposerContext = createContext<ComposerContextValue | undefined>(undefined)

/**
 * Owns the chat composer's context-item state: inspected elements and the
 * synced preview-URL reference. Lifted above the task-page layouts so a
 * mobile↔desktop swap doesn't drop the pending context.
 */
export function TaskComposerProvider({ children }: { children: ReactNode }) {
  const [contextItems, setContextItems] = useState<MessageContextInput[]>([])

  // Get current preview path from URL state
  const { previewPath } = usePreviewPath()

  // Sync preview path to context items whenever it changes
  useEffect(() => {
    if (!previewPath || previewPath === '/') {
      // Remove any existing preview URL context when preview is at root
      setContextItems((prev) => prev.filter((item) => item.type !== 'previewUrl'))
      return
    }

    // Add or update preview URL context
    setContextItems((prev) => {
      const existingIndex = prev.findIndex((item) => item.type === 'previewUrl')

      if (existingIndex !== -1) {
        // Update existing preview URL context
        return prev.map((item, idx) =>
          idx === existingIndex
            ? {
                type: 'previewUrl',
                referenceId: previewPath,
                data: { path: previewPath },
              }
            : item,
        )
      }

      // Add new preview URL context
      return [
        ...prev,
        {
          type: 'previewUrl',
          referenceId: previewPath,
          data: { path: previewPath },
        },
      ]
    })
  }, [previewPath])

  // Handler to toggle element in context (add if not present, remove if present)
  const toggleElementContext = useCallback(
    (element: ElementInfo) => {
      // Check if element already exists in context by componentId
      const existingIndex = contextItems.findIndex((item) => {
        if (item.type === 'element' && element.componentId) {
          return item.data.componentId === element.componentId
        }
        return false
      })

      if (existingIndex !== -1) {
        // Element exists - remove it
        setContextItems((prev) => prev.filter((_, index) => index !== existingIndex))
        toast.info('Element removed from context', {
          description: `${element.componentName || 'Element'} removed from context`,
        })
      } else {
        // Element doesn't exist - add it
        const newContext: MessageContextInput = {
          type: 'element',
          referenceId: element.componentId,
          data: {
            componentId: element.componentId,
            path: element.path,
            line: element.line,
            column: element.column,
            componentName: element.componentName,
          },
        }
        setContextItems((prev) => [...prev, newContext])
        toast.success('Element added to context', {
          description: `${element.componentName || 'Element'} added to context`,
        })
      }
    },
    [contextItems],
  )

  const value = useMemo<ComposerContextValue>(
    () => ({ contextItems, setContextItems, toggleElementContext }),
    [contextItems, toggleElementContext],
  )

  return <ComposerContext.Provider value={value}>{children}</ComposerContext.Provider>
}

export function useComposer(): ComposerContextValue {
  const context = useContext(ComposerContext)
  if (!context) {
    throw new Error('useComposer must be used within TaskComposerProvider')
  }
  return context
}
