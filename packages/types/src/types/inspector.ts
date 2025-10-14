/**
 * Inspector-related types for preview functionality
 * These types are shared between frontend and backend
 */

/** Source location information for inspector */
export interface SourceLocation {
  sourceId: string | null // data-lov-id: "file:line:column" format
  elementType: string | null // data-lov-name: element/component name
  componentPath: string | null // data-component-path: full source file path
  componentLine: number | null // data-component-line: line number
  componentFile: string | null // data-component-file: filename only
  componentName: string | null // data-component-name: element/component type
  componentContent: string | null // data-component-content: URL-encoded props JSON
}

/** Element information with parent chain for inspector */
export interface ElementInfo {
  sourceId: string | null // data-lov-id: "file:line:column" format
  elementType: string | null // data-lov-name: element/component name
  componentPath: string | null // data-component-path: full source file path
  componentLine: number | null // data-component-line: line number
  componentFile: string | null // data-component-file: filename only
  componentName: string | null // data-component-name: element/component type
  componentContent: string | null // data-component-content: URL-encoded props JSON
  parents: SourceLocation[]
}
