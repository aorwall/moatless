/**
 * Utilities for element inspection functionality
 */

import type { SourceLocation } from '../types/index.js'

/**
 * Generate a natural language prompt describing the selected component
 */
export function generateComponentList(element: SourceLocation): string {
  // Extract filename from path
  const filename = element.path.split('/').pop() || element.path

  // Build description with location
  const prompt = `Selected component <${element.componentName}> at ${filename}:${element.line}:${element.column}`

  return `${prompt}.`
}
