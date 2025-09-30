/* Preview Inspector Script - Injected by Preview Environment Service */
;(() => {
  const CONFIG = {
    HIGHLIGHT_COLOR: '#0da2e7',
    HIGHLIGHT_BG: '#0da2e71a',
    DEBOUNCE_MS: 10,
    TOOLTIP_OFFSET: 25,
    Z: 10000,
    MAX_TOOLTIP_WIDTH: 200,
    SCROLL_DEBOUNCE_MS: 420,
    ALLOWED_ORIGINS: ['*'], // Allow all origins for iframe communication
    FULL_WIDTH_DELTA: 12,
  }

  const ATTR_SELECTED = 'data-inspector-selected'
  const ATTR_HOVERED = 'data-inspector-hovered'

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
    ;['log', 'warn', 'error', 'info', 'debug'].forEach((level) => {
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
                } catch (e) {
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
    })
  }

  function hookRuntimeErrors() {
    window.addEventListener('error', (ev) => {
      sendToParent({
        type: 'console',
        level: 'error',
        message: ev.message + ' at ' + ev.filename + ':' + ev.lineno + ':' + ev.colno,
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
        message: 'Unhandled Promise Rejection: ' + String(ev.reason),
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
    if (window.location.hash && window.location.hash.startsWith('#/')) {
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
      if (window.location.hash && window.location.hash.startsWith('#/')) {
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
    // Scan DOM for elements with data-insp-path attribute
    const elementsWithInspPath = document.querySelectorAll('[data-insp-path]')
    const isAvailable = elementsWithInspPath.length > 0

    internalLog.log(
      '[Preview Inspector] Inspector availability check:',
      isAvailable
        ? `Found ${elementsWithInspPath.length} elements with data-insp-path`
        : 'No data-insp-path attributes found',
    )

    sendToParent({
      type: 'INSPECTOR_AVAILABILITY',
      payload: {
        isAvailable,
        elementCount: elementsWithInspPath.length,
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

    const style = document.createElement('style')
    style.textContent = `[${ATTR_HOVERED}],[${ATTR_SELECTED}]{position:relative !important}[${ATTR_HOVERED}]::before,[${ATTR_SELECTED}]::before{content:'';position:absolute;inset:0;outline:1px dashed ${CONFIG.HIGHLIGHT_COLOR};background:${CONFIG.HIGHLIGHT_BG};pointer-events:none;z-index:${CONFIG.Z}}[${ATTR_SELECTED}]::before{outline-offset:3px}`
    document.head.appendChild(style)

    let enabled = false
    let hovered

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

    const over = debounce((ev) => {
      if (!enabled) return
      const target = ev.target
      if (!(target instanceof HTMLElement)) return
      hovered?.removeAttribute(ATTR_HOVERED)
      hovered = target
      showTooltip(target)
      target.setAttribute(ATTR_HOVERED, 'true')
    }, CONFIG.DEBOUNCE_MS)

    const out = debounce(() => {
      hovered?.removeAttribute(ATTR_HOVERED)
      hideTooltip()
    }, CONFIG.DEBOUNCE_MS)

    // Extract source information from a single element's data-insp-path attribute
    // Format: "src/path/to/file.tsx:line:column:element"
    const extractSourceFromElement = (element) => {
      if (!element || !element.getAttribute) return null

      const inspPath = element.getAttribute('data-insp-path')
      if (!inspPath) return null

      // Parse the format: "path/to/file.tsx:line:column:element"
      const parts = inspPath.split(':')
      if (parts.length >= 3) {
        const filePath = parts[0]
        const lineNumber = Number.parseInt(parts[1], 10)
        const columnNumber = Number.parseInt(parts[2], 10)
        const elementTag = parts[3] || element.tagName?.toLowerCase() || null

        return {
          filePath,
          lineNumber: isNaN(lineNumber) ? null : lineNumber,
          columnNumber: isNaN(columnNumber) ? null : columnNumber,
          elementTag,
        }
      }
      return null
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
            'parents with data-insp-path',
          )
          return result
        }

        currentElement = currentElement.parentElement
        attempts++
      }

      internalLog.log('[Inspector Debug] No data-insp-path attribute found')
      return result
    }

    const click = (ev) => {
      if (!enabled) return
      ev.preventDefault()
      ev.stopPropagation()
      const target = ev.target
      if (!(target instanceof HTMLElement)) return

      // Clear any previously selected elements (single selection only)
      document
        .querySelectorAll(`[${ATTR_SELECTED}]`)
        .forEach((el) => el.removeAttribute(ATTR_SELECTED))

      target.setAttribute(ATTR_SELECTED, 'true')

      // Extract source location information from element and parent chain
      const chainInfo = extractParentChainInfo(target)

      // Send source location information with parent chain to parent
      sendToParent({
        type: 'ELEMENT_CLICKED',
        payload: chainInfo.element
          ? {
              filePath: chainInfo.element.filePath,
              lineNumber: chainInfo.element.lineNumber,
              columnNumber: chainInfo.element.columnNumber,
              elementTag: chainInfo.element.elementTag || target.tagName.toLowerCase(),
              parents: chainInfo.parents,
              timestamp: Date.now(),
            }
          : {
              filePath: null,
              lineNumber: null,
              columnNumber: null,
              elementTag: target.tagName.toLowerCase(),
              parents: [],
              timestamp: Date.now(),
            },
      })
    }

    const toggle = (state) => {
      enabled = state
      tooltip.style.display = 'none'
      if (enabled) {
        document.addEventListener('mouseover', over)
        document.addEventListener('mouseout', out)
        document.addEventListener('click', click, true)
        document.body.style.cursor = 'crosshair'
      } else {
        document.removeEventListener('mouseover', over)
        document.removeEventListener('mouseout', out)
        document.removeEventListener('click', click, true)
        document.body.style.cursor = ''
        document
          .querySelectorAll(`[${ATTR_HOVERED}], [${ATTR_SELECTED}]`)
          .forEach((el) => [ATTR_HOVERED, ATTR_SELECTED].forEach((a) => el.removeAttribute(a)))
      }
    }

    return { toggle }
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
      if (type === 'TOGGLE_SELECTOR' || type === 'SELECTOR_STATE_REQUEST') {
        internalLog.log('[Preview Inspector] Received message from parent:', ev.data)
      }

      switch (type) {
        case 'TOGGLE_SELECTOR':
          internalLog.log('[Preview Inspector] Toggling selector:', payload)
          selectorEnabled = Boolean(payload)
          overlay.toggle(selectorEnabled)
          break
        case 'SELECTOR_STATE_REQUEST':
          sendToParent({ type: 'SELECTOR_STATE_RESPONSE', payload: { isActive: selectorEnabled } })
          break
      }
    })

    internalLog.log('[Preview Inspector] Setting up hooks...')

    hookConsole()
    internalLog.log('[Preview Inspector] Console hooks installed')

    hookRuntimeErrors()
    internalLog.log('[Preview Inspector] Error hooks installed')

    hookFetch()
    internalLog.log('[Preview Inspector] Fetch hooks installed')

    hookScroll()
    internalLog.log('[Preview Inspector] Scroll hooks installed')

    hookNavigation()
    internalLog.log('[Preview Inspector] Navigation hooks installed')

    internalLog.log('[Preview Inspector] Checking inspector availability...')
    checkInspectorAvailability()

    internalLog.log('[Preview Inspector] Sending SCRIPT_LOADED message...')
    sendToParent({
      type: 'INSPECTOR_SCRIPT_LOADED',
      payload: { version: '2.0.0-server', taskId: '__TASK_ID__' },
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
