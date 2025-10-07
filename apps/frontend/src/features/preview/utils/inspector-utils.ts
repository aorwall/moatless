/**
 * Utilities for element inspection functionality
 */

import type { ElementInfo } from '../types'

/**
 * Format a source location for display
 */
export function formatSourceLocation(element: ElementInfo): string {
  if (!element.filePath) {
    return 'No source location available'
  }

  const line = element.lineNumber || 0
  const column = element.columnNumber || 0

  return `${element.filePath}:${line}:${column}`
}

/**
 * Check if element has valid source location
 */
export function hasSourceLocation(element: ElementInfo): boolean {
  return element.filePath !== null && element.lineNumber !== null
}

/**
 * Generate a list of relevant components for AI assistance
 */
export function generateComponentList(element: ElementInfo): string {
  const components: string[] = []

  // Add the selected element first (most specific)
  if (element.filePath) {
    components.push(`1. <${element.elementTag}> in @${element.filePath}:${element.lineNumber || 0}`)
  }

  // Add parent components (broader context)
  if (element.parents && element.parents.length > 0) {
    for (const [index, parent] of element.parents.entries()) {
      if (parent.filePath) {
        components.push(
          `${index + 2}. <${parent.elementTag || 'unknown'}> in @${parent.filePath}:${parent.lineNumber || 0}`,
        )
      }
    }
  }

  if (components.length === 0) {
    return 'No component hierarchy available. Enable code-inspector plugin to see file locations.'
  }

  return `Relevant components for this task:\n\n${components.join('\n')}`
}
