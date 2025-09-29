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
