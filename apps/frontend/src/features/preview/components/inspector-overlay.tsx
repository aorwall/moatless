import { type RefObject, useEffect } from 'react'
import type { ElementInfo } from '../types'

interface InspectorOverlayProps {
  iframeRef: RefObject<HTMLIFrameElement | null>
  isActive: boolean
  onElementSelected: (element: ElementInfo) => void
}

export function InspectorOverlay({
  iframeRef,
  isActive,
  onElementSelected,
}: InspectorOverlayProps) {
  useEffect(() => {
    const handleMessage = (event: MessageEvent) => {
      // Handle element clicked message from the backend-injected inspector script
      if (event.data.type === 'ELEMENT_CLICKED') {
        // Extract source location info from the payload with new attribute structure
        const payload = event.data.payload || {}

        // Validate required fields - only create ElementInfo if all required fields are present
        if (!(payload.file && payload.lineNumber && payload.path)) {
          // Element does not have required source location fields
          return
        }

        // Filter parents to only include those with required fields
        const validParents = (payload.parents || []).filter(
          (parent: unknown) =>
            parent &&
            typeof parent === 'object' &&
            'file' in parent &&
            'lineNumber' in parent &&
            'path' in parent &&
            parent.file &&
            parent.lineNumber &&
            parent.path,
        )

        // Convert to ElementInfo format with all LOV attributes, parent chain, and position
        const elementInfo: ElementInfo = {
          sourceId: payload.sourceId,
          componentName: payload.componentName,
          path: payload.path,
          lineNumber: payload.lineNumber,
          file: payload.file,
          elementType: payload.elementType,
          content: payload.content,
          className: payload.className,
          parents: validParents,
          elementRect: payload.elementRect,
        }

        onElementSelected(elementInfo)
      }
    }

    window.addEventListener('message', handleMessage)

    return () => {
      window.removeEventListener('message', handleMessage)
    }
  }, [onElementSelected])

  // Handle inspector activation state changes
  useEffect(() => {
    const iframe = iframeRef.current
    if (!iframe?.contentWindow) {
      return
    }

    // Send toggle message to the backend-injected inspector script
    iframe.contentWindow.postMessage(
      {
        type: 'TOGGLE_SELECTOR',
        payload: isActive,
      },
      '*',
    )
  }, [isActive, iframeRef])

  return null
}
