import { debounce, internalLogger, sendMessage } from './communication'
import { CONFIG } from './constants'
import type { ComponentInfo, ElementInspectorAPI } from './types'

/** Check if any elements with data-component-id exist and report availability */
export function checkAvailability(): void {
  const elements = document.querySelectorAll('[data-component-id]')
  const isAvailable = elements.length > 0

  internalLogger.log(
    '[Preview Inspector] Inspector availability check:',
    isAvailable
      ? `Found ${elements.length} elements with data-component-id`
      : 'No data-component-id attributes found',
  )

  sendMessage({
    type: 'INSPECTOR_AVAILABILITY',
    payload: { isAvailable, elementCount: elements.length, timestamp: Date.now() },
  })
}

/** Parse a component's data attributes into structured info */
function parseComponentInfo(element: Element | null): ComponentInfo | null {
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

  const lastColon = componentId.lastIndexOf(':')
  const secondLastColon = componentId.lastIndexOf(':', lastColon - 1)

  if (lastColon === -1 || secondLastColon === -1) {
    internalLogger.warn('[Preview Inspector] Invalid componentId format:', componentId)
    return null
  }

  const path = componentId.substring(0, secondLastColon)
  const line = Number.parseInt(componentId.substring(secondLastColon + 1, lastColon), 10)
  const column = Number.parseInt(componentId.substring(lastColon + 1), 10)

  if (Number.isNaN(line) || Number.isNaN(column)) {
    internalLogger.warn('[Preview Inspector] Invalid line/column in componentId:', componentId)
    return null
  }

  return { componentId, path, line, column, componentName }
}

/** Walk up the DOM tree to find the nearest element with data-component-id */
function findNearestComponent(target: HTMLElement): ComponentInfo | null {
  let current: HTMLElement | null = target
  let depth = 0
  const maxDepth = 10

  while (current && depth < maxDepth) {
    const info = parseComponentInfo(current)
    if (info) {
      internalLogger.log('[Inspector Debug] Found element with data-component-id')
      return info
    }
    current = current.parentElement
    depth++
  }

  internalLogger.log('[Inspector Debug] No data-component-id attribute found')
  return null
}

/** Find all elements matching a component ID */
function findElementsByComponentId(componentId: string): Element[] {
  const selector = `[data-component-id="${componentId.replace(/"/g, '\\"')}"]`
  return Array.from(document.querySelectorAll(selector))
}

/** Create an overlay div for selected elements */
function createSelectionOverlay(): HTMLDivElement {
  return Object.assign(document.createElement('div'), {
    className: 'inspector-selected-overlay',
    style: `position:fixed;z-index:${CONFIG.Z - 1};pointer-events:none;outline:1px dashed ${CONFIG.HIGHLIGHT_COLOR};background:${CONFIG.HIGHLIGHT_BG};display:none;`,
  })
}

/** Position an overlay div over a target element */
function positionOverlay(overlay: HTMLElement, target: Element, padding = 0): void {
  const rect = target.getBoundingClientRect()
  overlay.style.display = 'block'
  overlay.style.top = `${rect.top - padding}px`
  overlay.style.left = `${rect.left - padding}px`
  overlay.style.width = `${rect.width + padding * 2}px`
  overlay.style.height = `${rect.height + padding * 2}px`
}

/** Hide an overlay */
function hideOverlay(overlay: HTMLElement): void {
  overlay.style.display = 'none'
}

/** Create the element inspector with overlays and selection support */
export function createElementInspector(): ElementInspectorAPI {
  // Create tooltip element
  const tooltip = Object.assign(document.createElement('div'), {
    className: 'inspector-tooltip',
    style: `position:fixed;z-index:${CONFIG.Z};pointer-events:none;background:${CONFIG.HIGHLIGHT_COLOR};color:#fff;padding:4px 8px;border-radius:4px;font-weight:bold;white-space:nowrap;display:none;max-width:${CONFIG.MAX_TOOLTIP_WIDTH}px;font-size:14px;box-shadow:0 2px 4px rgba(0,0,0,.2)`,
  })
  document.body.appendChild(tooltip)

  // Create hover overlay element
  const hoverOverlay = Object.assign(document.createElement('div'), {
    className: 'inspector-hover-overlay',
    style: `position:fixed;z-index:${CONFIG.Z - 1};pointer-events:none;outline:1px dashed ${CONFIG.HIGHLIGHT_COLOR};background:${CONFIG.HIGHLIGHT_BG};display:none;`,
  })
  document.body.appendChild(hoverOverlay)

  let isActive = false
  let hoveredElement: HTMLElement | null = null
  const selectedElements = new Map<string, Element[]>()
  const selectionOverlays = new Map<string, HTMLDivElement[]>()
  let scrollBlockerStyle: HTMLStyleElement | null = null

  // Tooltip positioning
  const showTooltip = (target: HTMLElement) => {
    const { top, left, width } = target.getBoundingClientRect()
    tooltip.textContent = target.tagName.toLowerCase()
    tooltip.style.display = 'block'
    tooltip.style.opacity = '1'
    tooltip.style.left = `${left}px`
    tooltip.style.top = `${Math.max(0, top - CONFIG.TOOLTIP_OFFSET)}px`

    // If element is full-width, offset tooltip inward
    if (Math.abs(width - window.innerWidth) < 5) {
      tooltip.style.left = `${CONFIG.FULL_WIDTH_DELTA}px`
      tooltip.style.top = `${CONFIG.FULL_WIDTH_DELTA}px`
    }
  }

  const hideTooltip = () => {
    tooltip.style.display = 'none'
  }

  // Mouse event handlers (debounced)
  const handleMouseOver = debounce((event: MouseEvent) => {
    if (!isActive) {
      return
    }
    const target = document.elementFromPoint(event.clientX, event.clientY)
    if (target instanceof HTMLElement && hoveredElement !== target) {
      hoveredElement = target
      showTooltip(target)
      positionOverlay(hoverOverlay, target, 0)
    }
  }, CONFIG.DEBOUNCE_MS)

  const handleMouseOut = debounce(() => {
    hideTooltip()
    hideOverlay(hoverOverlay)
  }, CONFIG.DEBOUNCE_MS)

  // Prevent default interactions while selector is active
  const preventInteraction = (event: Event) => {
    if (isActive) {
      event.preventDefault()
      event.stopPropagation()
    }
    return false
  }

  const preventFocusInput = (event: Event) => {
    if (!isActive) {
      return
    }
    const target = event.target as HTMLElement | null
    if (
      target &&
      target instanceof HTMLElement &&
      ['input', 'textarea', 'select'].includes(target.tagName?.toLowerCase())
    ) {
      event.preventDefault()
    }
  }

  // Click handler - sends ELEMENT_CLICKED message
  const handleClick = (event: MouseEvent) => {
    if (!isActive) {
      return
    }
    event.preventDefault()
    event.stopPropagation()

    const target = document.elementFromPoint(event.clientX, event.clientY)
    if (!(target instanceof HTMLElement)) {
      return
    }

    const componentInfo = findNearestComponent(target)
    const rect = target.getBoundingClientRect()
    const elementRect = {
      top: rect.top,
      left: rect.left,
      width: rect.width,
      height: rect.height,
      bottom: rect.bottom,
      right: rect.right,
    }

    sendMessage({
      type: 'ELEMENT_CLICKED',
      payload: componentInfo
        ? {
            componentId: componentInfo.componentId,
            path: componentInfo.path,
            line: componentInfo.line,
            column: componentInfo.column,
            componentName: componentInfo.componentName,
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

  // Selection management
  const addToSelection = (componentId: string) => {
    if (selectedElements.has(componentId)) {
      internalLogger.log('[Inspector] ComponentId already selected:', componentId)
      return
    }

    const elements = findElementsByComponentId(componentId)
    if (elements.length === 0) {
      internalLogger.warn('[Inspector] Could not find any elements with componentId:', componentId)
      return
    }

    internalLogger.log(
      `[Inspector] Found ${elements.length} element(s) with componentId:`,
      componentId,
    )

    const overlays: HTMLDivElement[] = []
    for (const element of elements) {
      const overlay = createSelectionOverlay()
      document.body.appendChild(overlay)
      positionOverlay(overlay, element, 3)
      overlays.push(overlay)
    }

    selectedElements.set(componentId, elements)
    selectionOverlays.set(componentId, overlays)
    internalLogger.log(
      '[Inspector] Added to selection:',
      componentId,
      `(${elements.length} instances)`,
    )
  }

  const removeFromSelection = (componentId: string) => {
    const overlays = selectionOverlays.get(componentId)
    if (overlays) {
      for (const overlay of overlays) {
        overlay.remove()
      }
      selectionOverlays.delete(componentId)
    }
    selectedElements.delete(componentId)
    internalLogger.log('[Inspector] Removed from selection:', componentId)
  }

  const clearAllSelections = () => {
    for (const overlays of selectionOverlays.values()) {
      for (const overlay of overlays) {
        overlay.remove()
      }
    }
    selectedElements.clear()
    selectionOverlays.clear()
    internalLogger.log('[Inspector] Cleared all selections')
  }

  // Toggle selector mode
  const toggle = (active: boolean) => {
    isActive = active
    tooltip.style.display = 'none'

    if (isActive) {
      document.addEventListener('mouseover', handleMouseOver as EventListener)
      document.addEventListener('mouseout', handleMouseOut as EventListener)
      document.addEventListener('click', handleClick, true)
      document.addEventListener('mousedown', preventFocusInput, true)
      document.addEventListener('submit', preventInteraction, true)
      document.addEventListener('touchstart', preventInteraction, true)
      document.addEventListener('touchend', preventInteraction, true)
      document.body.style.cursor = 'crosshair'

      // Disable smooth scrolling while inspecting
      scrollBlockerStyle = document.createElement('style')
      scrollBlockerStyle.textContent = '* { scroll-behavior: auto !important; }'
      document.head.appendChild(scrollBlockerStyle)
    } else {
      document.removeEventListener('mouseover', handleMouseOver as EventListener)
      document.removeEventListener('mouseout', handleMouseOut as EventListener)
      document.removeEventListener('click', handleClick, true)
      document.removeEventListener('mousedown', preventFocusInput, true)
      document.removeEventListener('submit', preventInteraction, true)
      document.removeEventListener('touchstart', preventInteraction, true)
      document.removeEventListener('touchend', preventInteraction, true)
      document.body.style.cursor = ''

      if (scrollBlockerStyle) {
        scrollBlockerStyle.remove()
        scrollBlockerStyle = null
      }

      hideOverlay(hoverOverlay)

      // Hide (but don't remove) selection overlays
      for (const overlays of selectionOverlays.values()) {
        for (const overlay of overlays) {
          hideOverlay(overlay)
        }
      }

      hoveredElement = null
    }
  }

  // Update overlay positions on scroll/resize
  const updateOverlayPositions = debounce(() => {
    if (!isActive) {
      return
    }

    if (hoveredElement) {
      positionOverlay(hoverOverlay, hoveredElement, 0)
    }

    for (const [componentId, elements] of selectedElements.entries()) {
      const overlays = selectionOverlays.get(componentId)
      if (overlays && elements) {
        const len = Math.min(elements.length, overlays.length)
        for (let i = 0; i < len; i++) {
          const overlay = overlays[i]
          const element = elements[i]
          if (overlay && element) {
            positionOverlay(overlay, element, 3)
          }
        }
      }
    }
  }, CONFIG.DEBOUNCE_MS)

  window.addEventListener('scroll', updateOverlayPositions, { passive: true })
  window.addEventListener('resize', updateOverlayPositions, { passive: true })

  return { toggle, addToSelection, removeFromSelection, clearAllSelections }
}
