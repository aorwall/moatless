export interface SourceLocation {
  sourceId: string | null // data-lov-id: "file:line:column" format
  componentName: string | null // data-lov-name: component name
  path: string // data-component-path: full source file path (required)
  lineNumber: number // data-component-line: line number (required)
  file: string // data-component-file: filename only (required)
  elementType: string | null // data-component-name: element/HTML tag type
  content: string | null // text content from component props (children or text property)
  className: string | null // className from component props
}

export interface ElementRect {
  top: number
  left: number
  width: number
  height: number
  bottom: number
  right: number
}

export interface ElementInfo {
  sourceId: string | null // data-lov-id: "file:line:column" format
  componentName: string | null // data-lov-name: component name
  path: string // data-component-path: full source file path (required)
  lineNumber: number // data-component-line: line number (required)
  file: string // data-component-file: filename only (required)
  elementType: string | null // data-component-name: element/HTML tag type
  content: string | null // text content from component props (children or text property)
  className: string | null // className from component props
  parents: SourceLocation[]
  elementRect?: ElementRect // bounding rect for popover positioning
}

export interface PreviewProps {
  taskId: string
  onElementSelected?: (element: ElementInfo | null) => void
  onAddElementToContext?: (element: ElementInfo) => void
  className?: string
}

export type ScriptHealthStatus = 'not-injected' | 'injected-not-reachable' | 'injected-reachable'
