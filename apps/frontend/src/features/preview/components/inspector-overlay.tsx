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

        // Convert to ElementInfo format with all LOV attributes and parent chain
        const elementInfo: ElementInfo = {
          sourceId: payload.sourceId,
          componentName: payload.componentName,
          componentPath: payload.componentPath,
          componentLine: payload.componentLine,
          componentFile: payload.componentFile,
          elementType: payload.elementType,
          componentContent: payload.componentContent,
          parents: payload.parents || [],
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
