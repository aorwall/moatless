/**
 * Inspector-related types for preview functionality
 * These types are shared between frontend and backend
 */

/** Source location information for inspector */
export interface SourceLocation {
  sourceId: string | null // data-lov-id: "file:line:column" format
  componentName: string | null // data-lov-name: component name
  componentPath: string | null // data-component-path: full source file path
  componentLine: number | null // data-component-line: line number
  componentFile: string | null // data-component-file: filename only
  elementType: string | null // data-component-name: element/HTML tag type
  componentContent: string | null // data-component-content: URL-encoded props JSON
}

/** Element information with parent chain for inspector */
export interface ElementInfo {
  sourceId: string | null // data-lov-id: "file:line:column" format
  componentName: string | null // data-lov-name: component name
  componentPath: string | null // data-component-path: full source file path
  componentLine: number | null // data-component-line: line number
  componentFile: string | null // data-component-file: filename only
  elementType: string | null // data-component-name: element/HTML tag type
  componentContent: string | null // data-component-content: URL-encoded props JSON
  parents: SourceLocation[]
}
