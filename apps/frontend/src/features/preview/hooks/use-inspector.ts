import { useCallback } from 'react'
import type { ElementInfo } from '../types'
import { usePreview } from './use-preview'

export function useInspector() {
  const { isInspectorActive, selectedElement, toggleInspector, setSelectedElement } = usePreview()

  const handleElementSelected = useCallback(
    (element: ElementInfo | null) => {
      setSelectedElement(element)

      // Copy selector to clipboard
      if (element?.selector) {
        navigator.clipboard.writeText(element.selector).catch(() => {
          // Ignore clipboard errors
        })
      }
    },
    [setSelectedElement],
  )

  const clearSelection = useCallback(() => {
    setSelectedElement(null)
  }, [setSelectedElement])

  return {
    isActive: isInspectorActive,
    selectedElement,
    toggle: toggleInspector,
    selectElement: handleElementSelected,
    clearSelection,
  }
}
