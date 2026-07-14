import { internalLogger, sendMessage } from './communication'
import { installConsoleInterceptors } from './console-interceptor'
import { CONFIG } from './constants'
import { checkAvailability, createElementInspector } from './element-inspector'
import { installErrorHandlers } from './error-handler'
import { installNetworkMonitor } from './network-monitor'
import { installRouteTracker } from './route-tracker'
import { installScrollMonitor } from './scroll-monitor'
import type { ElementInspectorAPI, InspectorInboundMessageType } from './types'

/** The serverId placeholder - replaced at injection time */
const SERVER_ID = '__SERVER_ID__'

/** Wait for the DOM to be ready */
function waitForDom(): Promise<void> {
  return new Promise((resolve) => {
    if (document.readyState === 'loading') {
      document.addEventListener('DOMContentLoaded', () => resolve(), { once: true })
    } else {
      resolve()
    }
  })
}

/** Navigate to a path */
function navigate(target: unknown): void {
  internalLogger.log('[Preview Inspector] Navigating to:', target)
  try {
    const path =
      typeof target === 'object' && target !== null && 'path' in target
        ? (target as { path: string }).path
        : String(target)

    let currentPath = window.location.pathname + window.location.search
    if (window.location.hash?.startsWith('#/')) {
      currentPath = window.location.hash
    }

    if (currentPath !== path) {
      internalLogger.log('[Preview Inspector] Current path differs, navigating:', {
        from: currentPath,
        to: path,
      })
      if (path.startsWith('#')) {
        window.location.hash = path
      } else {
        window.location.href = path
      }
    } else {
      internalLogger.log('[Preview Inspector] Already at target path, skipping navigation')
    }
  } catch (error) {
    internalLogger.error('[Preview Inspector] Navigation failed:', error)
  }
}

/** Handle inbound messages from the parent window */
function handleMessage(
  event: MessageEvent,
  selectorState: { current: boolean },
  inspector: ElementInspectorAPI,
): void {
  const { type, payload } = (event.data as { type?: string; payload?: unknown }) ?? {}

  const knownTypes: InspectorInboundMessageType[] = [
    'TOGGLE_SELECTOR',
    'SELECTOR_STATE_REQUEST',
    'CHECK_AVAILABILITY_REQUEST',
    'CLEAR_SELECTION',
    'ADD_TO_SELECTION',
    'REMOVE_FROM_SELECTION',
    'NAVIGATE',
  ]

  if (knownTypes.includes(type as InspectorInboundMessageType)) {
    internalLogger.log('[Preview Inspector] Received message from parent:', event.data)
  }

  switch (type) {
    case 'TOGGLE_SELECTOR':
      internalLogger.log('[Preview Inspector] Toggling selector:', payload)
      selectorState.current = !!payload
      inspector.toggle(selectorState.current)
      break
    case 'SELECTOR_STATE_REQUEST':
      sendMessage({ type: 'SELECTOR_STATE_RESPONSE', payload: { isActive: selectorState.current } })
      break
    case 'CHECK_AVAILABILITY_REQUEST':
      internalLogger.log('[Preview Inspector] Checking availability on request')
      checkAvailability()
      break
    case 'CLEAR_SELECTION':
      internalLogger.log('[Preview Inspector] Clearing all selections')
      inspector.clearAllSelections()
      break
    case 'ADD_TO_SELECTION':
      if (typeof payload === 'object' && payload !== null && 'componentId' in payload) {
        internalLogger.log(
          '[Preview Inspector] Adding to selection:',
          (payload as { componentId: string }).componentId,
        )
        inspector.addToSelection(String((payload as { componentId: string }).componentId))
      }
      break
    case 'REMOVE_FROM_SELECTION':
      if (typeof payload === 'object' && payload !== null && 'componentId' in payload) {
        internalLogger.log(
          '[Preview Inspector] Removing from selection:',
          (payload as { componentId: string }).componentId,
        )
        inspector.removeFromSelection(String((payload as { componentId: string }).componentId))
      }
      break
    case 'NAVIGATE':
      navigate(payload)
      break
    default:
      break
  }
}

/** Bootstrap the inspector */
async function bootstrap(): Promise<void> {
  internalLogger.log('[Preview Inspector] Script starting to execute...')

  await waitForDom()
  internalLogger.log('[Preview Inspector] DOM is ready')

  const selectorState = { current: false }
  const inspector = createElementInspector()
  internalLogger.log('[Preview Inspector] Overlay built')

  window.__moatless = {
    toggleChat() {
      window.parent.postMessage({ type: 'TOGGLE_CHAT', source: 'preview-iframe' }, '*')
    },
    isAvailable() {
      return document.querySelectorAll('[data-component-id]').length > 0
    },
    isSelectorActive() {
      return selectorState.current
    },
  }

  window.addEventListener('message', (event) => {
    handleMessage(event, selectorState, inspector)
  })

  internalLogger.log('[Preview Inspector] Setting up hooks...')

  installConsoleInterceptors()
  internalLogger.log('[Preview Inspector] Console hooks installed')

  installErrorHandlers()
  internalLogger.log('[Preview Inspector] Error hooks installed')

  if (CONFIG.FEATURES.NETWORK_MONITORING) {
    installNetworkMonitor()
    internalLogger.log('[Preview Inspector] Fetch hooks installed')
  }

  if (CONFIG.FEATURES.SCROLL_MONITORING) {
    installScrollMonitor()
    internalLogger.log('[Preview Inspector] Scroll hooks installed')
  }

  installRouteTracker()
  internalLogger.log('[Preview Inspector] Navigation hooks installed')

  internalLogger.log('[Preview Inspector] Sending SCRIPT_LOADED message...')
  sendMessage({
    type: 'INSPECTOR_SCRIPT_LOADED',
    payload: { version: '2.0.0-server', serverId: SERVER_ID },
  })

  setInterval(() => {
    sendMessage({ type: 'HEARTBEAT', payload: { timestamp: Date.now() } })
  }, 5000)

  internalLogger.log('[Preview Inspector] Bootstrap complete!')
}

// Expose a global API for apps running inside the iframe to call
type MoatlessGlobal = {
  /** Collapse or expand the chat panel in the parent kiosk frame. */
  toggleChat: () => void
  /** Returns true if any elements with data-component-id exist on the page. */
  isAvailable: () => boolean
  /** Returns true if the element selector (click-to-inspect) mode is currently active. */
  isSelectorActive: () => boolean
}
declare global {
  interface Window {
    __moatless?: MoatlessGlobal
  }
}

bootstrap().catch((error) => {
  internalLogger.error('[Preview Inspector] Bootstrap failed:', error)
})
