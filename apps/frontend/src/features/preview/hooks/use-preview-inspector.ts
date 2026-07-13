import { type RefObject, useCallback, useEffect, useState } from 'react'
import type { ElementInfo } from '../types'

interface UsePreviewInspectorOptions {
  iframeRef: RefObject<HTMLIFrameElement | null>
  onElementSelected?: ((element: ElementInfo | null) => void) | undefined
  onToggleElementContext?: ((element: ElementInfo) => void) | undefined
}

/**
 * Element-inspector state and its postMessage bridge to the preview iframe:
 * activation toggle, availability handshake (CHECK_AVAILABILITY_REQUEST →
 * INSPECTOR_AVAILABILITY), selection overlay sync, and the iframe→parent
 * coordinate transform applied to selected elements.
 */
export function usePreviewInspector({
  iframeRef,
  onElementSelected,
  onToggleElementContext,
}: UsePreviewInspectorOptions) {
  const [isInspectorActive, setIsInspectorActive] = useState(false)
  const [isInspectorAvailable, setIsInspectorAvailable] = useState<boolean | null>(null)
  const [isPendingInspectorActivation, setIsPendingInspectorActivation] = useState(false)
  const [isInspectorDialogOpen, setIsInspectorDialogOpen] = useState(false)
  const [selectedComponentId, setSelectedComponentId] = useState<string | null>(null)

  // Function to request availability check from iframe
  const requestAvailabilityCheck = useCallback(() => {
    if (iframeRef.current?.contentWindow) {
      setIsPendingInspectorActivation(true)
      iframeRef.current.contentWindow.postMessage(
        {
          type: 'CHECK_AVAILABILITY_REQUEST',
        },
        '*',
      )
    }
  }, [iframeRef])

  // Function to toggle the element selector in the iframe
  const toggleSelector = useCallback(
    (enabled: boolean) => {
      if (iframeRef.current?.contentWindow) {
        iframeRef.current.contentWindow.postMessage(
          {
            type: 'TOGGLE_SELECTOR',
            payload: enabled,
          },
          '*',
        )
      }
    },
    [iframeRef],
  )

  // Function to clear all selections
  const clearSelection = useCallback(() => {
    if (iframeRef.current?.contentWindow) {
      iframeRef.current.contentWindow.postMessage(
        {
          type: 'CLEAR_SELECTION',
        },
        '*',
      )
    }
  }, [iframeRef])

  // Function to add element to selection overlay
  const addToSelection = useCallback(
    (componentId: string) => {
      if (iframeRef.current?.contentWindow) {
        iframeRef.current.contentWindow.postMessage(
          {
            type: 'ADD_TO_SELECTION',
            payload: { componentId },
          },
          '*',
        )
      }
    },
    [iframeRef],
  )

  // Function to remove element from selection overlay
  const removeFromSelection = useCallback(
    (componentId: string) => {
      if (iframeRef.current?.contentWindow) {
        iframeRef.current.contentWindow.postMessage(
          {
            type: 'REMOVE_FROM_SELECTION',
            payload: { componentId },
          },
          '*',
        )
      }
    },
    [iframeRef],
  )

  // Sync selector state with inspector active state
  useEffect(() => {
    toggleSelector(isInspectorActive)
  }, [isInspectorActive, toggleSelector])

  // Handle inspector availability message from the iframe
  const handleInspectorAvailability = useCallback(
    (payload: { isAvailable: boolean; elementCount?: number }) => {
      setIsInspectorAvailable(payload.isAvailable)

      // If user was waiting for availability check, auto-enable or show dialog
      if (isPendingInspectorActivation) {
        setIsPendingInspectorActivation(false)

        if (payload.isAvailable) {
          // Auto-enable inspector
          setIsInspectorActive(true)
        } else {
          // Show unavailable dialog
          setIsInspectorDialogOpen(true)
        }
      }
    },
    [isPendingInspectorActivation],
  )

  const toggleInspector = useCallback(() => {
    setIsInspectorActive((prev) => {
      // If disabling inspector, clear selection
      if (prev) {
        setSelectedComponentId(null)
      }
      return !prev
    })
  }, [])

  // Element selected in the InspectorOverlay
  const handleElementSelected = useCallback(
    (element: ElementInfo) => {
      // Transform iframe-relative coordinates to parent-window coordinates
      let transformedElement = element
      if (element.elementRect && iframeRef.current) {
        const iframeRect = iframeRef.current.getBoundingClientRect()
        transformedElement = {
          ...element,
          elementRect: {
            top: element.elementRect.top + iframeRect.top,
            left: element.elementRect.left + iframeRect.left,
            width: element.elementRect.width,
            height: element.elementRect.height,
            bottom: element.elementRect.bottom + iframeRect.top,
            right: element.elementRect.right + iframeRect.left,
          },
        }
      }

      // Check if clicking the same element again (to toggle it off)
      const isSameElement =
        selectedComponentId && transformedElement.componentId === selectedComponentId

      if (isSameElement) {
        // Clicking same element again - remove selection overlay and from context
        removeFromSelection(transformedElement.componentId)
        setSelectedComponentId(null)
      } else {
        // Different element - clear old selection and add new one
        clearSelection()
        addToSelection(transformedElement.componentId)
        setSelectedComponentId(transformedElement.componentId)
      }

      // Notify parent of selection (optional callback)
      onElementSelected?.(transformedElement)
      // Automatically toggle element in context
      onToggleElementContext?.(transformedElement)
    },
    [
      iframeRef,
      selectedComponentId,
      removeFromSelection,
      clearSelection,
      addToSelection,
      onElementSelected,
      onToggleElementContext,
    ],
  )

  /** Reload: deactivate the inspector but keep the availability result. */
  const deactivateInspector = useCallback(() => {
    setIsInspectorActive(false)
    setSelectedComponentId(null)
  }, [])

  /** Server changed or stopped: full inspector reset. */
  const resetInspector = useCallback(() => {
    setIsInspectorActive(false)
    setIsInspectorAvailable(null)
    setSelectedComponentId(null)
  }, [])

  return {
    isInspectorActive,
    isInspectorAvailable,
    isInspectorDialogOpen,
    setIsInspectorDialogOpen,
    requestAvailabilityCheck,
    handleInspectorAvailability,
    toggleInspector,
    handleElementSelected,
    deactivateInspector,
    resetInspector,
  }
}
