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
        // Extract source location info from the payload with new tagger attribute structure
        const payload = event.data.payload || {}

        // Validate required fields - only create ElementInfo if all required fields are present
        if (
          !(payload.componentId && payload.path && payload.line != null && payload.column != null)
        ) {
          // Element does not have required source location fields
          return
        }

        // Convert to ElementInfo format with tagger attributes and position
        const elementInfo: ElementInfo = {
          componentId: payload.componentId,
          path: payload.path,
          line: payload.line,
          column: payload.column,
          componentName: payload.componentName,
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
