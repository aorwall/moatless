/**
 * Utilities for element inspection functionality
 */

import type { ElementInfo } from '../types/index.js'

/**
 * Format a source location for display
 */
export function formatSourceLocation(element: ElementInfo): string {
  return `${element.path}:${element.line}:${element.column}`
}

/**
 * Generate a natural language prompt describing the selected component
 */
export function generateComponentList(element: ElementInfo): string {
  const elementName = element.componentName || 'unknown'

  // Extract filename from path
  const filename = element.path.split('/').pop() || element.path

  // Build description with location
  const prompt = `Selected component <${elementName}> at ${filename}:${element.line}:${element.column}`

  return `${prompt}.`
}
