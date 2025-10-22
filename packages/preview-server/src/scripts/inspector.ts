/* Preview Inspector Script - Injected by Preview Environment Service */

import { CONFIG } from './config.js'
import type {
  ElementRect,
  IncomingMessage,
  Logger,
  MessagePayload,
  MessageWithSource,
  OverlayController,
  SourceInfo,
} from './types.js'

// IIFE to avoid polluting global scope
;(() => {
  // Store original console methods for internal logging
  const originalConsole: Logger = {
    log: console.log,
    warn: console.warn,
    error: console.error,
    info: console.info,
    debug: console.debug,
  }

  // Helper for internal logging that doesn't trigger hooks
  const internalLog: Logger = {
    log: (...args: unknown[]): void => originalConsole.log(...args),
    warn: (...args: unknown[]): void => originalConsole.warn(...args),
    error: (...args: unknown[]): void => originalConsole.error(...args),
    info: (...args: unknown[]): void => originalConsole.info(...args),
    debug: (...args: unknown[]): void => originalConsole.debug(...args),
  }

  const sendToParent = (payload: MessagePayload): void => {
    try {
      // Always ensure source is set for all messages
      const messageWithSource: MessageWithSource = { ...payload, source: 'preview-iframe' }

      // Use internal logging to prevent infinite recursion (exclude HEARTBEAT from logs)
      if (payload.type !== 'HEARTBEAT') {
        internalLog.log('[Preview Inspector] Sending message to parent:', messageWithSource)
      }

      window.parent.postMessage(messageWithSource, '*')
    } catch (err) {
      internalLog.error('preview-inspector: could not post to parent', err)
    }
  }

  const debounce = <T extends (...args: unknown[]) => void>(
    fn: T,
    wait: number,
  ): ((...args: Parameters<T>) => void) => {
    let handle: ReturnType<typeof setTimeout> | undefined
    return (...args: Parameters<T>): void => {
      if (handle !== undefined) {
        clearTimeout(handle)
      }
      handle = setTimeout(() => fn(...args), wait)
    }
  }

  function hookConsole(): void {
    for (const level of ['log', 'warn', 'error', 'info', 'debug'] as const) {
      console[level] = (...args: unknown[]): void => {
        // Always call original first to show in browser console
        originalConsole[level].apply(console, args)

        const stack =
          level === 'error' || level === 'warn'
            ? new Error().stack?.split('\n').slice(2).join('\n')
            : undefined
        sendToParent({
          type: 'console',
          level,
          message: args
            .map((a) => {
              if (typeof a === 'object') {
                try {
                  return JSON.stringify(a, null, 2)
                } catch {
                  return String(a)
                }
              }
              return String(a)
            })
            .join(' '),
          args,
          stack,
        })
      }
    }
  }

  function hookRuntimeErrors(): void {
    window.addEventListener('error', (ev: ErrorEvent) => {
      sendToParent({
        type: 'console',
        level: 'error',
        message: `${ev.message} at ${ev.filename}:${ev.lineno}:${ev.colno}`,
        args: [ev.message],
        stack: ev.error?.stack,
      })
    })
    window.addEventListener('unhandledrejection', (ev: PromiseRejectionEvent) => {
      const stack =
        ev.reason?.stack || (ev.reason instanceof Error ? String(ev.reason.stack) : undefined)
      sendToParent({
        type: 'console',
        level: 'error',
        message: `Unhandled Promise Rejection: ${String(ev.reason)}`,
        args: [String(ev.reason)],
        stack,
      })
    })
  }

  function hookFetch(): void {
    const original = window.fetch
    window.fetch = async (...args: Parameters<typeof fetch>): Promise<Response> => {
      const started = performance.now()
      try {
        const resp = await original(...args)
        sendToParent({
          type: 'NETWORK_REQUEST',
          payload: {
            url: (typeof args[0] === 'string' ? args[0] : args[0]?.url) ?? resp.url,
            method: args[1]?.method ?? 'GET',
            status: resp.status,
            statusText: resp.statusText,
            duration: performance.now() - started,
            timestamp: new Date().toISOString(),
            origin: location.origin,
          },
        })
        return resp
      } catch (err) {
        const error = err as Error
        sendToParent({
          type: 'NETWORK_REQUEST',
          payload: {
            url: typeof args[0] === 'string' ? args[0] : (args[0] as Request).url,
            method: args[1]?.method ?? 'GET',
            error: { message: error.message, stack: error.stack },
            duration: performance.now() - started,
            timestamp: new Date().toISOString(),
            origin: location.origin,
          },
        })
        throw err
      }
    }
  }

  function hookScroll(): void {
    const throttled = debounce(() => {
      sendToParent({
        type: 'SCROLL_POSITION',
        payload: {
          scrollY: window.scrollY,
          scrollHeight: document.documentElement.scrollHeight,
          clientHeight: window.innerHeight,
          timestamp: Date.now(),
        },
      })
    }, CONFIG.DEBOUNCE_MS)
    window.addEventListener('scroll', throttled, { passive: true })
  }

  function hookNavigation(): void {
    let currentPath = window.location.pathname + window.location.search

    // For hash-based routing, use the hash as the path
    if (window.location.hash?.startsWith('#/')) {
      currentPath = window.location.hash.substring(1) // Remove the # character
    }

    // Send initial route
    sendToParent({
      type: 'ROUTE_CHANGE',
      payload: {
        path: currentPath,
        timestamp: Date.now(),
      },
    })

    // Hook into browser navigation events
    const checkRouteChange = (): void => {
      let newPath = window.location.pathname + window.location.search

      // For hash-based routing, use the hash as the path
      if (window.location.hash?.startsWith('#/')) {
        newPath = window.location.hash.substring(1) // Remove the # character
      }

      if (newPath !== currentPath) {
        currentPath = newPath
        sendToParent({
          type: 'ROUTE_CHANGE',
          payload: {
            path: currentPath,
            timestamp: Date.now(),
          },
        })
      }
    }

    // Listen for browser navigation events
    window.addEventListener('popstate', checkRouteChange)

    // Hook into pushState and replaceState for client-side navigation
    const originalPushState = history.pushState
    const originalReplaceState = history.replaceState

    history.pushState = (...args: Parameters<typeof history.pushState>): void => {
      originalPushState.apply(history, args)
      setTimeout(checkRouteChange, 0) // Use setTimeout to ensure the URL has changed
    }

    history.replaceState = (...args: Parameters<typeof history.replaceState>): void => {
      originalReplaceState.apply(history, args)
      setTimeout(checkRouteChange, 0)
    }

    // Also listen for hash changes (for hash-based routing)
    window.addEventListener('hashchange', checkRouteChange)
  }

  function checkInspectorAvailability(): void {
    // Scan DOM for elements with data-component-id attribute
    const elementsWithSourceData = document.querySelectorAll('[data-component-id]')
    const isAvailable = elementsWithSourceData.length > 0

    internalLog.log(
      '[Preview Inspector] Inspector availability check:',
      isAvailable
        ? `Found ${elementsWithSourceData.length} elements with data-component-id`
        : 'No data-component-id attributes found',
    )

    sendToParent({
      type: 'INSPECTOR_AVAILABILITY',
      payload: {
        isAvailable,
        elementCount: elementsWithSourceData.length,
        timestamp: Date.now(),
      },
    })
  }

  function buildOverlay(): OverlayController {
    const tooltip = Object.assign(document.createElement('div'), {
      className: 'inspector-tooltip',
      style: `position:fixed;z-index:${CONFIG.Z};pointer-events:none;background:${CONFIG.HIGHLIGHT_COLOR};color:#fff;padding:4px 8px;border-radius:4px;font-weight:bold;white-space:nowrap;display:none;max-width:${CONFIG.MAX_TOOLTIP_WIDTH}px;font-size:14px;box-shadow:0 2px 4px rgba(0,0,0,.2)`,
    })
    document.body.appendChild(tooltip)

    // Create external overlay elements instead of CSS attributes
    const hoverOverlay = Object.assign(document.createElement('div'), {
      className: 'inspector-hover-overlay',
      style: `position:fixed;z-index:${CONFIG.Z - 1};pointer-events:none;outline:1px dashed ${CONFIG.HIGHLIGHT_COLOR};background:${CONFIG.HIGHLIGHT_BG};display:none;`,
    })
    document.body.appendChild(hoverOverlay)

    let enabled = false
    let hovered: HTMLElement | null = null

    // Track selected componentId and all its instances
    const selectedElements = new Map<string, HTMLElement[]>() // componentId -> array of elements
    const selectedOverlays = new Map<string, HTMLElement[]>() // componentId -> array of overlays

    const showTooltip = (el: HTMLElement): void => {
      const { top, left, width } = el.getBoundingClientRect()
      tooltip.textContent = el.tagName.toLowerCase()
      tooltip.style.display = 'block'
      tooltip.style.opacity = '1'
      tooltip.style.left = `${left}px`
      tooltip.style.top = `${Math.max(0, top - CONFIG.TOOLTIP_OFFSET)}px`
      if (Math.abs(width - window.innerWidth) < 5) {
        tooltip.style.left = `${CONFIG.FULL_WIDTH_DELTA}px`
        tooltip.style.top = `${CONFIG.FULL_WIDTH_DELTA}px`
      }
    }

    const hideTooltip = (): void => {
      tooltip.style.display = 'none'
    }

    // Position overlay element over target element
    const positionOverlay = (overlay: HTMLElement, el: HTMLElement, outlineOffset = 0): void => {
      const rect = el.getBoundingClientRect()
      overlay.style.display = 'block'
      overlay.style.top = `${rect.top - outlineOffset}px`
      overlay.style.left = `${rect.left - outlineOffset}px`
      overlay.style.width = `${rect.width + outlineOffset * 2}px`
      overlay.style.height = `${rect.height + outlineOffset * 2}px`
    }

    const hideOverlay = (overlay: HTMLElement): void => {
      overlay.style.display = 'none'
    }

    const over = debounce((ev: MouseEvent) => {
      if (!enabled) {
        return
      }
      // Use the actual topmost element under cursor instead of event target
      const target = document.elementFromPoint(ev.clientX, ev.clientY)
      if (!(target instanceof HTMLElement)) {
        return
      }
      // Prevent re-highlighting the same element
      if (hovered === target) {
        return
      }
      hovered = target
      showTooltip(target)
      positionOverlay(hoverOverlay, target, 0)
    }, CONFIG.DEBOUNCE_MS)

    const out = debounce(() => {
      hideTooltip()
      hideOverlay(hoverOverlay)
    }, CONFIG.DEBOUNCE_MS)

    // Extract source information from a single element's data attributes
    // Reads component tagging attributes for source tracking
    const extractSourceFromElement = (element: Element): SourceInfo | null => {
      if (!element?.getAttribute) {
        return null
      }

      const componentId = element.getAttribute('data-component-id')
      if (!componentId) {
        return null
      }

      const componentName = element.getAttribute('data-component-name')
      if (!componentName) {
        return null
      }

      // Parse componentId format: "path:line:column"
      const lastColonIndex = componentId.lastIndexOf(':')
      const secondLastColonIndex = componentId.lastIndexOf(':', lastColonIndex - 1)

      if (lastColonIndex === -1 || secondLastColonIndex === -1) {
        internalLog.warn('[Preview Inspector] Invalid componentId format:', componentId)
        return null
      }

      const path = componentId.substring(0, secondLastColonIndex)
      const line = Number.parseInt(
        componentId.substring(secondLastColonIndex + 1, lastColonIndex),
        10,
      )
      const column = Number.parseInt(componentId.substring(lastColonIndex + 1), 10)

      if (Number.isNaN(line) || Number.isNaN(column)) {
        internalLog.warn('[Preview Inspector] Invalid line/column in componentId:', componentId)
        return null
      }

      return {
        componentId,
        path,
        line,
        column,
        componentName,
      }
    }

    // Extract source information from element or its parent chain
    const findElementWithSource = (element: Element): SourceInfo | null => {
      let currentElement: Element | null = element
      let attempts = 0
      const maxSearchAttempts = 10

      // Find the first element with data-component-id (could be the element itself or a parent)
      while (currentElement && attempts < maxSearchAttempts) {
        const sourceInfo = extractSourceFromElement(currentElement)
        if (sourceInfo) {
          internalLog.log('[Inspector Debug] Found element with data-component-id')
          return sourceInfo
        }

        currentElement = currentElement.parentElement
        attempts++
      }

      internalLog.log('[Inspector Debug] No data-component-id attribute found')
      return null
    }

    // Helper: Create a new overlay element for a selection
    const createSelectionOverlay = (): HTMLElement => {
      return Object.assign(document.createElement('div'), {
        className: 'inspector-selected-overlay',
        style: `position:fixed;z-index:${CONFIG.Z - 1};pointer-events:none;outline:1px dashed ${CONFIG.HIGHLIGHT_COLOR};background:${CONFIG.HIGHLIGHT_BG};display:none;`,
      })
    }

    // Helper: Find ALL elements in DOM by componentId
    const findAllElementsByComponentId = (componentId: string): HTMLElement[] => {
      // Query for all elements with matching data-component-id attribute
      const selector = `[data-component-id="${componentId.replace(/"/g, '\\"')}"]`
      return Array.from(document.querySelectorAll<HTMLElement>(selector))
    }

    // Add element to selection by componentId (highlights all instances)
    const addToSelection = (componentId: string): void => {
      // Skip if already selected
      if (selectedElements.has(componentId)) {
        internalLog.log('[Inspector] ComponentId already selected:', componentId)
        return
      }

      // Find ALL elements in the DOM with this componentId
      const elements = findAllElementsByComponentId(componentId)
      if (elements.length === 0) {
        internalLog.warn('[Inspector] Could not find any elements with componentId:', componentId)
        return
      }

      internalLog.log(
        `[Inspector] Found ${elements.length} element(s) with componentId:`,
        componentId,
      )

      // Create overlays for each instance
      const overlays: HTMLElement[] = []
      for (const element of elements) {
        const overlay = createSelectionOverlay()
        document.body.appendChild(overlay)
        positionOverlay(overlay, element, 3)
        overlays.push(overlay)
      }

      // Store all elements and overlays for this componentId
      selectedElements.set(componentId, elements)
      selectedOverlays.set(componentId, overlays)

      internalLog.log(
        '[Inspector] Added to selection:',
        componentId,
        `(${elements.length} instances)`,
      )
    }

    // Remove element from selection by componentId (removes all overlays for this componentId)
    const removeFromSelection = (componentId: string): void => {
      const overlays = selectedOverlays.get(componentId)
      if (overlays) {
        // Remove all overlay elements from DOM
        for (const overlay of overlays) {
          overlay.remove()
        }
        selectedOverlays.delete(componentId)
      }

      selectedElements.delete(componentId)
      internalLog.log('[Inspector] Removed from selection:', componentId)
    }

    // Clear all selections
    const clearAllSelections = (): void => {
      // Remove all overlay elements from DOM
      for (const overlays of selectedOverlays.values()) {
        for (const overlay of overlays) {
          overlay.remove()
        }
      }

      selectedElements.clear()
      selectedOverlays.clear()
      internalLog.log('[Inspector] Cleared all selections')
    }

    // Prevent all interactions except selection
    const preventInteraction = (ev: Event): boolean => {
      if (!enabled) {
        return false
      }
      ev.preventDefault()
      ev.stopPropagation()
      return false
    }

    // Prevent input focus during selection
    const preventInputFocus = (ev: Event): void => {
      if (!enabled) {
        return
      }
      const target = ev.target
      if (
        target &&
        target instanceof HTMLElement &&
        ['input', 'textarea', 'select'].includes(target.tagName?.toLowerCase())
      ) {
        ev.preventDefault()
      }
    }

    const click = (ev: MouseEvent): void => {
      if (!enabled) {
        return
      }
      ev.preventDefault()
      ev.stopPropagation()
      // Use the actual topmost element under cursor, same as hover detection
      const target = document.elementFromPoint(ev.clientX, ev.clientY)
      if (!(target instanceof HTMLElement)) {
        return
      }

      // Extract source location information from element
      const sourceInfo = findElementWithSource(target)

      // Get element bounding rect for popover positioning
      const rect = target.getBoundingClientRect()
      const elementRect: ElementRect = {
        top: rect.top,
        left: rect.left,
        width: rect.width,
        height: rect.height,
        bottom: rect.bottom,
        right: rect.right,
      }

      // Send source location information with position to parent
      // Parent will decide whether to add/remove from selection
      // and send back ADD_TO_SELECTION or REMOVE_FROM_SELECTION messages
      sendToParent({
        type: 'ELEMENT_CLICKED',
        payload: sourceInfo
          ? {
              componentId: sourceInfo.componentId,
              path: sourceInfo.path,
              line: sourceInfo.line,
              column: sourceInfo.column,
              componentName: sourceInfo.componentName,
              elementRect,
              timestamp: Date.now(),
            }
          : {
              componentId: null,
              path: null,
              line: null,
              column: null,
              componentName: target.tagName.toLowerCase(),
              elementRect,
              timestamp: Date.now(),
            },
      })
    }

    let scrollStyle: HTMLStyleElement | null = null

    const toggle = (state: boolean): void => {
      enabled = state
      tooltip.style.display = 'none'
      if (enabled) {
        document.addEventListener('mouseover', over as EventListener)
        document.addEventListener('mouseout', out as EventListener)
        document.addEventListener('click', click, true)
        document.addEventListener('mousedown', preventInputFocus, true)
        document.addEventListener('submit', preventInteraction, true)
        document.addEventListener('touchstart', preventInteraction, true)
        document.addEventListener('touchend', preventInteraction, true)
        document.body.style.cursor = 'crosshair'

        // Add scroll-behavior style
        scrollStyle = document.createElement('style')
        scrollStyle.textContent = '* { scroll-behavior: auto !important; }'
        document.head.appendChild(scrollStyle)
      } else {
        document.removeEventListener('mouseover', over as EventListener)
        document.removeEventListener('mouseout', out as EventListener)
        document.removeEventListener('click', click, true)
        document.removeEventListener('mousedown', preventInputFocus, true)
        document.removeEventListener('submit', preventInteraction, true)
        document.removeEventListener('touchstart', preventInteraction, true)
        document.removeEventListener('touchend', preventInteraction, true)
        document.body.style.cursor = ''

        // Remove scroll-behavior style
        if (scrollStyle) {
          scrollStyle.remove()
          scrollStyle = null
        }

        // Hide overlays when disabled
        hideOverlay(hoverOverlay)
        // Hide all selection overlays
        for (const overlays of selectedOverlays.values()) {
          for (const overlay of overlays) {
            hideOverlay(overlay)
          }
        }
        hovered = null
      }
    }

    // Update overlay positions on scroll and resize
    const updateOverlayPositions = (): void => {
      if (!enabled) {
        return
      }
      if (hovered) {
        positionOverlay(hoverOverlay, hovered, 0)
      }
      // Update positions for all selected overlays
      for (const [componentId, elements] of selectedElements.entries()) {
        const overlays = selectedOverlays.get(componentId)
        if (overlays && elements) {
          // Update each overlay to match its corresponding element
          for (let i = 0; i < elements.length && i < overlays.length; i++) {
            positionOverlay(overlays[i], elements[i], 3)
          }
        }
      }
    }

    const debouncedUpdate = debounce(updateOverlayPositions, CONFIG.DEBOUNCE_MS)
    window.addEventListener('scroll', debouncedUpdate as EventListener, { passive: true })
    window.addEventListener('resize', debouncedUpdate as EventListener, { passive: true })

    return {
      toggle,
      addToSelection,
      removeFromSelection,
      clearAllSelections,
    }
  }

  // Wait for DOM to be ready before bootstrapping
  function waitForDOM(): Promise<void> {
    return new Promise((resolve) => {
      if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', () => resolve(), { once: true })
      } else {
        resolve()
      }
    })
  }
  // Handle navigation message
  function handleNavigate(payload: IncomingMessage['payload']): void {
    internalLog.log('[Preview Inspector] Navigating to:', payload)
    try {
      const newPath =
        typeof payload === 'object' && payload !== null && 'path' in payload
          ? payload.path
          : String(payload)
      const currentPath = window.location.pathname + window.location.search

      // Only navigate if we're not already at the target path
      if (currentPath !== newPath) {
        internalLog.log('[Preview Inspector] Current path differs, navigating:', {
          from: currentPath,
          to: newPath,
        })
        // Use window.location.href for universal navigation
        // This works for all frameworks (Next.js, React Router, Vue Router, etc.)
        // Causes a full page reload but works cross-origin
        window.location.href = newPath
      } else {
        internalLog.log('[Preview Inspector] Already at target path, skipping navigation')
      }
    } catch (error) {
      internalLog.error('[Preview Inspector] Navigation failed:', error)
    }
  }
  // Handle incoming messages from parent window
  function handleIncomingMessage(
    ev: MessageEvent<IncomingMessage>,
    selectorEnabledRef: { current: boolean },
    overlay: ReturnType<typeof buildOverlay>,
  ): void {
    const { type, payload } = ev.data ?? {}

    // Only log known Preview Inspector message types
    const knownTypes = [
      'TOGGLE_SELECTOR',
      'SELECTOR_STATE_REQUEST',
      'CHECK_AVAILABILITY_REQUEST',
      'CLEAR_SELECTION',
      'ADD_TO_SELECTION',
      'REMOVE_FROM_SELECTION',
      'NAVIGATE',
    ]
    if (knownTypes.includes(type ?? '')) {
      internalLog.log('[Preview Inspector] Received message from parent:', ev.data)
    }

    switch (type) {
      case 'TOGGLE_SELECTOR':
        internalLog.log('[Preview Inspector] Toggling selector:', payload)
        selectorEnabledRef.current = Boolean(payload)
        overlay.toggle(selectorEnabledRef.current)
        break
      case 'SELECTOR_STATE_REQUEST':
        sendToParent({
          type: 'SELECTOR_STATE_RESPONSE',
          payload: { isActive: selectorEnabledRef.current },
        })
        break
      case 'CHECK_AVAILABILITY_REQUEST':
        internalLog.log('[Preview Inspector] Checking availability on request')
        checkInspectorAvailability()
        break
      case 'CLEAR_SELECTION':
        internalLog.log('[Preview Inspector] Clearing all selections')
        overlay.clearAllSelections()
        break
      case 'ADD_TO_SELECTION':
        if (typeof payload === 'object' && payload !== null && 'componentId' in payload) {
          internalLog.log('[Preview Inspector] Adding to selection:', payload.componentId)
          overlay.addToSelection(String(payload.componentId))
        }
        break
      case 'REMOVE_FROM_SELECTION':
        if (typeof payload === 'object' && payload !== null && 'componentId' in payload) {
          internalLog.log('[Preview Inspector] Removing from selection:', payload.componentId)
          overlay.removeFromSelection(String(payload.componentId))
        }
        break
      case 'NAVIGATE':
        handleNavigate(payload)
        break
    }
  }
  // Bootstrap everything
  ;(async (): Promise<void> => {
    internalLog.log('[Preview Inspector] Script starting to execute...')

    // Wait for DOM to be ready
    await waitForDOM()
    internalLog.log('[Preview Inspector] DOM is ready')

    const selectorEnabledRef = { current: false } // Track selector state globally
    const overlay = buildOverlay()
    internalLog.log('[Preview Inspector] Overlay built')

    window.addEventListener('message', (ev: MessageEvent<IncomingMessage>) => {
      handleIncomingMessage(ev, selectorEnabledRef, overlay)
    })

    internalLog.log('[Preview Inspector] Setting up hooks...')

    hookConsole()
    internalLog.log('[Preview Inspector] Console hooks installed')

    hookRuntimeErrors()
    internalLog.log('[Preview Inspector] Error hooks installed')

    if (CONFIG.FEATURES.NETWORK_MONITORING) {
      hookFetch()
      internalLog.log('[Preview Inspector] Fetch hooks installed')
    }

    if (CONFIG.FEATURES.SCROLL_MONITORING) {
      hookScroll()
      internalLog.log('[Preview Inspector] Scroll hooks installed')
    }

    hookNavigation()
    internalLog.log('[Preview Inspector] Navigation hooks installed')

    internalLog.log('[Preview Inspector] Sending SCRIPT_LOADED message...')
    sendToParent({
      type: 'INSPECTOR_SCRIPT_LOADED',
      payload: { version: '2.0.0-server', serverId: '__SERVER_ID__' },
    })

    // Start sending heartbeat messages every 5 seconds
    setInterval(() => {
      sendToParent({ type: 'HEARTBEAT', payload: { timestamp: Date.now() } })
    }, 5000)

    internalLog.log('[Preview Inspector] Bootstrap complete!')
  })().catch((err: unknown) => {
    internalLog.error('[Preview Inspector] Bootstrap failed:', err)
  })
})()
