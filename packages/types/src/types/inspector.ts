/**
 * Inspector-related types for preview functionality
 * These types are shared between frontend and backend
 */

/** Source location information for inspector */
export interface SourceLocation {
  filePath: string | null
  lineNumber: number | null
  columnNumber: number | null
  elementTag: string | null
}

/** Element information with parent chain for inspector */
export interface ElementInfo {
  filePath: string | null
  lineNumber: number | null
  columnNumber: number | null
  elementTag: string
  parents: SourceLocation[]
}
