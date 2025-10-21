/**
 * Utilities for element inspection functionality
 */

import type { SourceLocation } from '../types/inspector.js'

/**
 * Generate a natural language prompt describing the selected component
 */
export function generateComponentList(element: SourceLocation): string {
  const prompt = `Selected component <${element.componentName}> at ${element.path}:${element.line}:${element.column}`

  return `${prompt}.`
}
