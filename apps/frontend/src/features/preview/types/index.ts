export interface SourceLocation {
  filePath: string | null
  lineNumber: number | null
  columnNumber: number | null
  elementTag: string | null
}

export interface ElementInfo {
  filePath: string | null
  lineNumber: number | null
  columnNumber: number | null
  elementTag: string
  parents: SourceLocation[]
}

export interface PreviewProps {
  taskId: string
  onElementSelected?: (element: ElementInfo | null) => void
  className?: string
}

export type ScriptHealthStatus = 'not-injected' | 'injected-not-reachable' | 'injected-reachable'
