import { type RefObject, useEffect, useRef } from 'react'
import { usePreviewStore } from '../stores/preview-store'
import type { ElementInfo } from '../types'
import { INSPECTOR_SCRIPT } from '../utils/inspector-script'

interface InspectorOverlayProps {
  iframeRef: RefObject<HTMLIFrameElement | null>
  onElementSelected?: (element: ElementInfo | null) => void
}

export function InspectorOverlay({ iframeRef, onElementSelected }: InspectorOverlayProps) {
  const injectedRef = useRef(false)
  const isInspectorActive = usePreviewStore((state) => state.isInspectorActive)
  const setSelectedElement = usePreviewStore((state) => state.setSelectedElement)

  useEffect(() => {
    const iframe = iframeRef.current
    if (!iframe) return

    const injectScript = () => {
      try {
        const doc = iframe.contentDocument
        if (!doc || injectedRef.current) return

        const script = doc.createElement('script')
        script.textContent = INSPECTOR_SCRIPT
        doc.head.appendChild(script)
        injectedRef.current = true

        // Immediately activate if inspector is already active
        if (isInspectorActive && iframe.contentWindow) {
          setTimeout(() => {
            iframe.contentWindow?.postMessage({ type: 'INSPECTOR_ACTIVATE', active: true }, '*')
          }, 100)
        }
      } catch (error) {
        console.warn('Cannot inject inspector script due to CORS:', error)
      }
    }

    const handleMessage = (event: MessageEvent) => {
      if (event.data.type === 'ELEMENT_SELECTED') {
        const elementInfo = event.data.elementInfo as ElementInfo
        setSelectedElement(elementInfo)
        onElementSelected?.(elementInfo)

        // Copy displayText (readable selector) to clipboard
        if (elementInfo?.displayText) {
          navigator.clipboard
            .writeText(elementInfo.displayText)
            .then(() => {
              console.log('Selector copied to clipboard:', elementInfo.displayText)
            })
            .catch((err) => {
              console.warn('Failed to copy to clipboard:', err)
            })
        }
      } else if (event.data.type === 'INSPECTOR_READY') {
        // Inspector script is ready, activate if needed
        if (isInspectorActive && iframe.contentWindow) {
          iframe.contentWindow.postMessage({ type: 'INSPECTOR_ACTIVATE', active: true }, '*')
        }
      }
    }

    // Try to inject immediately if iframe is loaded
    if (iframe.contentDocument?.readyState === 'complete') {
      injectScript()
    }

    // Also listen for load event
    iframe.addEventListener('load', injectScript)
    window.addEventListener('message', handleMessage)

    return () => {
      iframe.removeEventListener('load', injectScript)
      window.removeEventListener('message', handleMessage)

      // Send disable message
      if (iframe.contentWindow) {
        iframe.contentWindow.postMessage({ type: 'DISABLE_INSPECTOR' }, '*')
      }

      injectedRef.current = false
    }
  }, [iframeRef, onElementSelected, setSelectedElement, isInspectorActive])

  // Handle inspector activation state changes
  useEffect(() => {
    const iframe = iframeRef.current
    if (!iframe?.contentWindow || !injectedRef.current) return

    iframe.contentWindow.postMessage({ type: 'INSPECTOR_ACTIVATE', active: isInspectorActive }, '*')
  }, [isInspectorActive, iframeRef])

  return null
}
