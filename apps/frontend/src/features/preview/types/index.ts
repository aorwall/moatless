export interface SourceLocation {
  sourceId: string | null // data-lov-id: "file:line:column" format
  componentName: string | null // data-lov-name: component name
  componentPath: string | null // data-component-path: full source file path
  componentLine: number | null // data-component-line: line number
  componentFile: string | null // data-component-file: filename only
  elementType: string | null // data-component-name: element/HTML tag type
  componentContent: string | null // data-component-content: URL-encoded props JSON
}

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

export interface PreviewProps {
  taskId: string
  onElementSelected?: (element: ElementInfo | null) => void
  className?: string
}

export type ScriptHealthStatus = 'not-injected' | 'injected-not-reachable' | 'injected-reachable'
