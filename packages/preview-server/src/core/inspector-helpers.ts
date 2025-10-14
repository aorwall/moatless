/**
 * Utilities for element inspection functionality
 */

import type { ElementInfo } from '../types/index.js'

/**
 * Format a source location for display
 */
export function formatSourceLocation(element: ElementInfo): string {
  return `${element.path}:${element.lineNumber}`
}

/**
 * Generate a natural language prompt describing the selected component
 */
export function generateComponentList(element: ElementInfo): string {
  // Get element name
  const elementName = element.componentName || element.elementType || 'unknown'

  // Build main description
  let prompt = `Selected component (HTML element <${elementName}>) at line ${element.lineNumber} in ${element.file}`

  // Add first parent if exists
  if (element.parents?.[0]) {
    const parent = element.parents[0]
    const parentName = parent.componentName || parent.elementType || 'unknown'
    prompt += `, found inside component (HTML element <${parentName}>) at line ${parent.lineNumber} in ${parent.file}`
  }

  // Add text if exists (truncate if too long)
  if (element.content) {
    const truncatedText =
      element.content.length > 50 ? `${element.content.substring(0, 50)}...` : element.content
    prompt += `. Has text "${truncatedText}"`
  }

  // Add classes if exist
  if (element.className) {
    prompt += element.content ? ' and' : '. Has'
    prompt += ` classes "${element.className}"`
  }

  return `${prompt}.`
}
