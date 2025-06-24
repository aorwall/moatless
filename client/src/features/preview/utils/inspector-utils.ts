/**
 * Utilities for element inspection functionality
 */

/**
 * Generate a human-readable CSS selector for an element
 */
export function createReadableSelector(element: Element): string {
  // Priority: ID > unique class > tag + structural position
  if (element.id) {
    return `#${element.id}`
  }

  const tagName = element.tagName.toLowerCase()

  if (element.className && typeof element.className === 'string') {
    const classes = element.className
      .split(' ')
      .filter((c) => c.trim() && !c.includes(':'))
      .slice(0, 2) // Limit to first 2 classes for readability

    if (classes.length > 0) {
      return `${tagName}.${classes.join('.')}`
    }
  }

  // Generate nth-child selector
  const parent = element.parentElement
  if (parent) {
    const index = Array.from(parent.children).indexOf(element) + 1
    const parentSelector = parent.tagName.toLowerCase()
    return `${parentSelector} > ${tagName}:nth-child(${index})`
  }

  return tagName
}

/**
 * Generate a unique CSS selector path for an element
 */
export function generateSelector(element: Element): string {
  const path: string[] = []
  let current: Element | null = element

  while (current && current.tagName && current.tagName !== 'HTML') {
    let selector = current.tagName.toLowerCase()

    if (current.id) {
      selector = `#${current.id}`
      path.unshift(selector)
      break // ID is unique, no need to go further
    }

    if (current.className && typeof current.className === 'string') {
      const classes = current.className.split(' ').filter((c) => c.trim() && !c.includes(':'))
      if (classes.length > 0) {
        selector += `.${classes[0]}`
      }
    }

    // Add nth-child if needed for uniqueness
    const parent = current.parentElement
    if (parent) {
      const siblings = Array.from(parent.children).filter(
        (child) => child.tagName === current!.tagName,
      )
      if (siblings.length > 1) {
        const index = siblings.indexOf(current) + 1
        selector += `:nth-child(${index})`
      }
    }

    path.unshift(selector)
    current = current.parentElement
  }

  return path.join(' > ')
}

/**
 * Get the element path as an array of selectors
 */
export function getElementPath(element: Element): string[] {
  const path: string[] = []
  let current: Element | null = element

  while (current && current.tagName && current.tagName !== 'HTML') {
    const selector = createReadableSelector(current)
    path.unshift(selector)
    current = current.parentElement
  }

  return path
}

/**
 * Extract all attributes from an element
 */
export function getElementAttributes(element: Element): Record<string, string> {
  const attributes: Record<string, string> = {}

  for (let i = 0; i < element.attributes.length; i++) {
    const attr = element.attributes[i]
    attributes[attr.name] = attr.value
  }

  return attributes
}

/**
 * Get computed styles for an element
 */
export function getComputedStyles(element: Element): Record<string, string> {
  const computed = window.getComputedStyle(element)

  return {
    display: computed.display,
    position: computed.position,
    width: computed.width,
    height: computed.height,
    color: computed.color,
    backgroundColor: computed.backgroundColor,
    fontSize: computed.fontSize,
    fontFamily: computed.fontFamily,
    padding: computed.padding,
    margin: computed.margin,
    border: computed.border,
  }
}

/**
 * Create a complete ElementInfo object from a DOM element
 */
export function createElementInfo(element: Element): any {
  const rect = element.getBoundingClientRect()
  const displayText = createReadableSelector(element)

  return {
    tagName: element.tagName.toLowerCase(),
    className: element.className || '',
    id: element.id || '',
    textContent: (element.textContent || '').substring(0, 100).trim(),
    selector: generateSelector(element),
    displayText,
    elementPath: getElementPath(element),
    attributes: getElementAttributes(element),
    computedStyles: getComputedStyles(element),
    boundingRect: {
      top: rect.top,
      left: rect.left,
      width: rect.width,
      height: rect.height,
      x: rect.x,
      y: rect.y,
    },
  }
}
