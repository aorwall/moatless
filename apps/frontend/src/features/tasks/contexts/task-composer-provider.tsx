import type { MessageContextInput } from '@moatless/api-client'
import { createContext, type ReactNode, useContext, useState } from 'react'
import { toast } from 'sonner'
import { usePreviewPath } from '@/features/preview/hooks/use-preview-view'
import type { ElementInfo } from '@/features/preview/types'

interface ComposerContextValue {
  contextItems: MessageContextInput[]
  setContextItems: (items: MessageContextInput[]) => void
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
  const [manualContextItems, setManualContextItems] = useState<MessageContextInput[]>([])

  // Get current preview path from URL state
  const { previewPath } = usePreviewPath()

  const previewContext: MessageContextInput | null =
    previewPath && previewPath !== '/'
      ? {
          type: 'previewUrl',
          referenceId: previewPath,
          data: { path: previewPath },
        }
      : null
  const contextItems = previewContext ? [...manualContextItems, previewContext] : manualContextItems

  const setContextItems = (items: MessageContextInput[]) => {
    setManualContextItems(items.filter((item) => item.type !== 'previewUrl'))
  }

  // Handler to toggle element in context (add if not present, remove if present)
  const toggleElementContext = (element: ElementInfo) => {
    // Check if element already exists in context by componentId
    const existingIndex = manualContextItems.findIndex((item) => {
      if (item.type === 'element' && element.componentId) {
        return item.data.componentId === element.componentId
      }
      return false
    })

    if (existingIndex !== -1) {
      // Element exists - remove it
      setManualContextItems((items) => items.filter((_, index) => index !== existingIndex))
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
      setManualContextItems((items) => [...items, newContext])
      toast.success('Element added to context', {
        description: `${element.componentName || 'Element'} added to context`,
      })
    }
  }

  const value: ComposerContextValue = { contextItems, setContextItems, toggleElementContext }

  return <ComposerContext value={value}>{children}</ComposerContext>
}

export function useComposer(): ComposerContextValue {
  const context = useContext(ComposerContext)
  if (!context) {
    throw new Error('useComposer must be used within TaskComposerProvider')
  }
  return context
}
