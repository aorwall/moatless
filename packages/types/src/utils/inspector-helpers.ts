/**
 * Utilities for element inspection functionality
 */

import type { ElementInfo } from '../types/inspector.js'

/**
 * Format a source location for display
 */
export function formatSourceLocation(element: ElementInfo): string {
  if (!element.componentPath) {
    return 'No source location available'
  }

  const line = element.componentLine || 0

  return `${element.componentPath}:${line}`
}

/**
 * Check if element has valid source location
 */
export function hasSourceLocation(element: ElementInfo): boolean {
  return element.componentPath !== null && element.componentLine !== null
}

/**
 * Parse component props from URL-encoded JSON
 */
function parseComponentProps(content: string | null): Record<string, unknown> | null {
  if (!content) {
    return null
  }
  try {
    const decoded = decodeURIComponent(content)
    return JSON.parse(decoded) as Record<string, unknown>
  } catch {
    return null
  }
}

/**
 * Generate a natural language prompt describing the selected component
 */
export function generateComponentList(element: ElementInfo): string {
  if (!(element.componentFile && element.componentLine)) {
    return 'No component location available. Enable code-inspector plugin to see file locations.'
  }

  // Parse props to extract className and text
  const props = parseComponentProps(element.componentContent)
  const className = props?.['className'] as string | undefined
  const text =
    (props?.['children'] as string | undefined) || (props?.['text'] as string | undefined)

  // Get element name
  const elementName = element.componentName || element.elementType || 'unknown'

  // Build main description
  let prompt = `Selected component (HTML element <${elementName}>) at line ${element.componentLine} in ${element.componentFile}`

  // Add first parent if exists
  if (
    element.parents?.[0]?.componentFile &&
    element.parents[0].componentFile &&
    element.parents[0].componentLine
  ) {
    const parent = element.parents[0]
    const parentName = parent.componentName || parent.elementType || 'unknown'
    prompt += `, found inside component (HTML element <${parentName}>) at line ${parent.componentLine} in ${parent.componentFile}`
  }

  // Add text if exists (truncate if too long)
  if (text && typeof text === 'string') {
    const truncatedText = text.length > 50 ? `${text.substring(0, 50)}...` : text
    prompt += `. Has text "${truncatedText}"`
  }

  // Add classes if exist
  if (className && typeof className === 'string') {
    prompt += text ? ' and' : '. Has'
    prompt += ` classes "${className}"`
  }

  return `${prompt}.`
}
