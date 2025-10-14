export interface SourceLocation {
  sourceId: string | null // data-lov-id: "file:line:column" format
  elementType: string | null // data-lov-name: element/component name
  componentPath: string | null // data-component-path: full source file path
  componentLine: number | null // data-component-line: line number
  componentFile: string | null // data-component-file: filename only
  componentName: string | null // data-component-name: element/component type
  componentContent: string | null // data-component-content: URL-encoded props JSON
}

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

export interface PreviewProps {
  taskId: string
  onElementSelected?: (element: ElementInfo | null) => void
  className?: string
}

export type ScriptHealthStatus = 'not-injected' | 'injected-not-reachable' | 'injected-reachable'
