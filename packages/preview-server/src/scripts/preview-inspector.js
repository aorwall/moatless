/* Preview Inspector Script - Injected by Preview Environment Service */
;(() => {
  const CONFIG = {
    HIGHLIGHT_COLOR: '#0da2e7',
    HIGHLIGHT_BG: '#0da2e71a',
    DEBOUNCE_MS: 10,
    TOOLTIP_OFFSET: 25,
    Z: 2147483647,
    MAX_TOOLTIP_WIDTH: 200,
    SCROLL_DEBOUNCE_MS: 420,
    ALLOWED_ORIGINS: ['*'], // Allow all origins for iframe communication
    FULL_WIDTH_DELTA: 12,
    FEATURES: {
      NETWORK_MONITORING: false, // Disable network request monitoring
      SCROLL_MONITORING: false, // Disable scroll position monitoring
    },
  }

  // Store original console methods for internal logging
  const originalConsole = {
    log: console.log,
    warn: console.warn,
    error: console.error,
    info: console.info,
    debug: console.debug,
  }

  // Helper for internal logging that doesn't trigger hooks
  const internalLog = {
    log: (...args) => originalConsole.log(...args),
    warn: (...args) => originalConsole.warn(...args),
    error: (...args) => originalConsole.error(...args),
    info: (...args) => originalConsole.info(...args),
    debug: (...args) => originalConsole.debug(...args),
  }

  const sendToParent = (payload) => {
    try {
      // Always ensure source is set for all messages
      const messageWithSource = { ...payload, source: 'preview-iframe' }

      // Use internal logging to prevent infinite recursion (exclude HEARTBEAT from logs)
      if (payload.type !== 'HEARTBEAT') {
        internalLog.log('[Preview Inspector] Sending message to parent:', messageWithSource)
      }

      window.parent.postMessage(messageWithSource, '*')
    } catch (err) {
      internalLog.error('preview-inspector: could not post to parent', err)
    }
  }

  const debounce = (fn, wait) => {
    let handle
    return (...args) => {
      clearTimeout(handle)
      handle = setTimeout(() => fn(...args), wait)
    }
  }

  function hookConsole() {
    for (const level of ['log', 'warn', 'error', 'info', 'debug']) {
      console[level] = (...args) => {
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
                } catch (_e) {
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

  function hookRuntimeErrors() {
    window.addEventListener('error', (ev) => {
      sendToParent({
        type: 'console',
        level: 'error',
        message: `${ev.message} at ${ev.filename}:${ev.lineno}:${ev.colno}`,
        args: [ev.message],
        stack: ev.error?.stack,
      })
    })
    window.addEventListener('unhandledrejection', (ev) => {
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

  function hookFetch() {
    const original = window.fetch
    window.fetch = async (...args) => {
      const started = performance.now()
      try {
        const resp = await original(...args)
        sendToParent({
          type: 'NETWORK_REQUEST',
          payload: {
            url: args[0] ?? resp.url,
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
        sendToParent({
          type: 'NETWORK_REQUEST',
          payload: {
            url: args[0],
            method: args[1]?.method ?? 'GET',
            error: { message: err.message, stack: err.stack },
            duration: performance.now() - started,
            timestamp: new Date().toISOString(),
            origin: location.origin,
          },
        })
        throw err
      }
    }
  }

  function hookScroll() {
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

  function hookNavigation() {
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
    const checkRouteChange = () => {
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

    history.pushState = (...args) => {
      originalPushState.apply(history, args)
      setTimeout(checkRouteChange, 0) // Use setTimeout to ensure the URL has changed
    }

    history.replaceState = (...args) => {
      originalReplaceState.apply(history, args)
      setTimeout(checkRouteChange, 0)
    }

    // Also listen for hash changes (for hash-based routing)
    window.addEventListener('hashchange', checkRouteChange)
  }

  function checkInspectorAvailability() {
    // Scan DOM for elements with data-lov-id attribute
    const elementsWithSourceData = document.querySelectorAll('[data-lov-id]')
    const isAvailable = elementsWithSourceData.length > 0

    internalLog.log(
      '[Preview Inspector] Inspector availability check:',
      isAvailable
        ? `Found ${elementsWithSourceData.length} elements with data-lov-id`
        : 'No data-lov-id attributes found',
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

  function buildOverlay() {
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

    const selectedOverlay = Object.assign(document.createElement('div'), {
      className: 'inspector-selected-overlay',
      style: `position:fixed;z-index:${CONFIG.Z - 1};pointer-events:none;outline:1px dashed ${CONFIG.HIGHLIGHT_COLOR};background:${CONFIG.HIGHLIGHT_BG};display:none;`,
    })
    document.body.appendChild(selectedOverlay)

    let enabled = false
    let hovered
    let selected

    const showTooltip = (el) => {
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

    const hideTooltip = () => {
      tooltip.style.display = 'none'
    }

    // Position overlay element over target element
    const positionOverlay = (overlay, el, outlineOffset = 0) => {
      const rect = el.getBoundingClientRect()
      overlay.style.display = 'block'
      overlay.style.top = `${rect.top - outlineOffset}px`
      overlay.style.left = `${rect.left - outlineOffset}px`
      overlay.style.width = `${rect.width + outlineOffset * 2}px`
      overlay.style.height = `${rect.height + outlineOffset * 2}px`
    }

    const hideOverlay = (overlay) => {
      overlay.style.display = 'none'
    }

    const over = debounce((ev) => {
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

    // Parse component content from URL-encoded JSON
    const parseComponentContent = (encodedContent) => {
      if (!encodedContent) {
        return { text: null, className: null }
      }
      try {
        const decoded = decodeURIComponent(encodedContent)
        const props = JSON.parse(decoded)

        // Extract text from children or text property
        const text = props.children || props.text || null
        // Ensure text is a string if it exists
        const textValue = text && typeof text === 'string' ? text : null

        // Extract className
        const className =
          props.className && typeof props.className === 'string' ? props.className : null

        return { text: textValue, className }
      } catch {
        return { text: null, className: null }
      }
    }

    // Extract source information from a single element's data attributes
    // Reads all LOV (Locality Of Variable) attributes for component source tracking
    const extractSourceFromElement = (element) => {
      if (!element?.getAttribute) {
        return null
      }

      const sourceId = element.getAttribute('data-lov-id')
      if (!sourceId) {
        return null
      }

      // Extract all source location attributes
      const componentName = element.getAttribute('data-lov-name')
      const path = element.getAttribute('data-component-path')
      const lineNumberStr = element.getAttribute('data-component-line')
      const file = element.getAttribute('data-component-file')
      const elementType = element.getAttribute('data-component-name')
      const contentAttr = element.getAttribute('data-component-content')

      const lineNumber = lineNumberStr ? Number.parseInt(lineNumberStr, 10) : null

      // Parse content to extract text and className
      const { text, className } = parseComponentContent(contentAttr)

      return {
        sourceId,
        componentName,
        path,
        lineNumber: Number.isNaN(lineNumber) ? null : lineNumber,
        file,
        elementType,
        content: text,
        className,
      }
    }

    // Extract source information from element and its parent chain
    const extractParentChainInfo = (element, maxParents = 3) => {
      const result = {
        element: null,
        parents: [],
      }

      // First, find the element with data-insp-path (could be the element itself or a parent)
      let currentElement = element
      let attempts = 0
      const maxSearchAttempts = 10

      // Find the first element with data-insp-path
      while (currentElement && attempts < maxSearchAttempts) {
        const sourceInfo = extractSourceFromElement(currentElement)
        if (sourceInfo) {
          result.element = sourceInfo

          // Now collect parent elements with data-insp-path
          let parentElement = currentElement.parentElement
          let parentCount = 0

          while (parentElement && parentCount < maxParents) {
            const parentInfo = extractSourceFromElement(parentElement)
            if (parentInfo) {
              result.parents.push(parentInfo)
              parentCount++
            }
            parentElement = parentElement.parentElement
          }

          internalLog.log(
            '[Inspector Debug] Found element and',
            result.parents.length,
            'parents with data-lov-id',
          )
          return result
        }

        currentElement = currentElement.parentElement
        attempts++
      }

      internalLog.log('[Inspector Debug] No data-lov-id attribute found')
      return result
    }

    // Prevent all interactions except selection
    const preventInteraction = (ev) => {
      if (!enabled) {
        return
      }
      ev.preventDefault()
      ev.stopPropagation()
      return false
    }

    // Prevent input focus during selection
    const preventInputFocus = (ev) => {
      if (!enabled) {
        return
      }
      const target = ev.target
      if (target && ['input', 'textarea', 'select'].includes(target.tagName?.toLowerCase())) {
        ev.preventDefault()
      }
    }

    const click = (ev) => {
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

      // Update selected element and overlay
      selected = target
      positionOverlay(selectedOverlay, target, 3)

      // Extract source location information from element and parent chain
      const chainInfo = extractParentChainInfo(target)

      // Get element bounding rect for popover positioning
      const rect = target.getBoundingClientRect()
      const elementRect = {
        top: rect.top,
        left: rect.left,
        width: rect.width,
        height: rect.height,
        bottom: rect.bottom,
        right: rect.right,
      }

      // Send source location information with parent chain and position to parent
      sendToParent({
        type: 'ELEMENT_CLICKED',
        payload: chainInfo.element
          ? {
              sourceId: chainInfo.element.sourceId,
              componentName: chainInfo.element.componentName,
              path: chainInfo.element.path,
              lineNumber: chainInfo.element.lineNumber,
              file: chainInfo.element.file,
              elementType: chainInfo.element.elementType,
              content: chainInfo.element.content,
              className: chainInfo.element.className,
              parents: chainInfo.parents,
              elementRect,
              timestamp: Date.now(),
            }
          : {
              sourceId: null,
              componentName: null,
              path: null,
              lineNumber: null,
              file: null,
              elementType: target.tagName.toLowerCase(),
              content: null,
              className: null,
              parents: [],
              elementRect,
              timestamp: Date.now(),
            },
      })
    }

    let scrollStyle = null

    const toggle = (state) => {
      enabled = state
      tooltip.style.display = 'none'
      if (enabled) {
        document.addEventListener('mouseover', over)
        document.addEventListener('mouseout', out)
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
        document.removeEventListener('mouseover', over)
        document.removeEventListener('mouseout', out)
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
        hideOverlay(selectedOverlay)
        hovered = null
        selected = null
      }
    }

    // Update overlay positions on scroll and resize
    const updateOverlayPositions = () => {
      if (!enabled) {
        return
      }
      if (hovered) {
        positionOverlay(hoverOverlay, hovered, 0)
      }
      if (selected) {
        positionOverlay(selectedOverlay, selected, 3)
      }
    }

    const debouncedUpdate = debounce(updateOverlayPositions, CONFIG.DEBOUNCE_MS)
    window.addEventListener('scroll', debouncedUpdate, { passive: true })
    window.addEventListener('resize', debouncedUpdate, { passive: true })

    const clearSelection = () => {
      hideOverlay(selectedOverlay)
      selected = null
    }

    return { toggle, clearSelection }
  }

  // Wait for DOM to be ready before bootstrapping
  function waitForDOM() {
    return new Promise((resolve) => {
      if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', resolve, { once: true })
      } else {
        resolve()
      }
    })
  }
  // Bootstrap everything
  ;(async () => {
    internalLog.log('[Preview Inspector] Script starting to execute...')

    // Wait for DOM to be ready
    await waitForDOM()
    internalLog.log('[Preview Inspector] DOM is ready')

    let selectorEnabled = false // Track selector state globally
    const overlay = buildOverlay()
    internalLog.log('[Preview Inspector] Overlay built')

    window.addEventListener('message', (ev) => {
      const { type, payload } = ev.data ?? {}

      // Only log known Preview Inspector message types
      if (
        type === 'TOGGLE_SELECTOR' ||
        type === 'SELECTOR_STATE_REQUEST' ||
        type === 'CHECK_AVAILABILITY_REQUEST' ||
        type === 'CLEAR_SELECTION' ||
        type === 'NAVIGATE'
      ) {
        internalLog.log('[Preview Inspector] Received message from parent:', ev.data)
      }

      switch (type) {
        case 'TOGGLE_SELECTOR':
          internalLog.log('[Preview Inspector] Toggling selector:', payload)
          selectorEnabled = Boolean(payload)
          overlay.toggle(selectorEnabled)
          break
        case 'SELECTOR_STATE_REQUEST':
          sendToParent({
            type: 'SELECTOR_STATE_RESPONSE',
            payload: { isActive: selectorEnabled },
          })
          break
        case 'CHECK_AVAILABILITY_REQUEST':
          internalLog.log('[Preview Inspector] Checking availability on request')
          checkInspectorAvailability()
          break
        case 'CLEAR_SELECTION':
          internalLog.log('[Preview Inspector] Clearing selection')
          overlay.clearSelection()
          break
        case 'NAVIGATE':
          internalLog.log('[Preview Inspector] Navigating to:', payload)
          try {
            const newPath = payload.path || payload
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
          break
      }
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
  })().catch((err) => {
    internalLog.error('[Preview Inspector] Bootstrap failed:', err)
  })
})()
