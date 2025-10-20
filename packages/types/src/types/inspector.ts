/**
 * Inspector-related types for preview functionality
 * These types are shared between frontend and backend
 */

/** Source location information for inspector */
export interface SourceLocation {
  componentId: string // data-component-id: "path:line:column" format (e.g., "src/components/ui/button.tsx:15:3")
  path: string // Parsed from componentId: file path
  line: number // Parsed from componentId: line number
  column: number // Parsed from componentId: column number
  componentName: string // data-component-name: component/element name (e.g., "Button", "div")
}

/** Element information for inspector */
export interface ElementInfo extends SourceLocation {}
