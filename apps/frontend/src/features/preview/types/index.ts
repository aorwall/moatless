import type { ServerEntry } from '@moatless/types'

export interface SourceLocation {
  componentId: string // data-component-id: "path:line:column" format (e.g., "src/components/ui/button.tsx:15:3")
  path: string // Parsed from componentId: file path
  line: number // Parsed from componentId: line number
  column: number // Parsed from componentId: column number
  componentName: string // data-component-name: component/element name (e.g., "Button", "div")
}

export interface ElementRect {
  top: number
  left: number
  width: number
  height: number
  bottom: number
  right: number
}

export interface ElementInfo extends SourceLocation {
  elementRect?: ElementRect // bounding rect for popover positioning
}

export interface PreviewProps {
  taskId: string
  environmentStatus?: 'not_created' | 'initializing' | 'ready' | 'stopped' | 'removing' | 'error'
  onElementSelected?: (element: ElementInfo | null) => void
  onToggleElementContext?: (element: ElementInfo) => void
  onNavigationReady?: (navigate: (path: string) => void) => void
  onTroubleshootRequest?: (reason: string) => void
  /** Dynamic server list for multi-server tab rendering */
  servers?: ServerEntry[]
  /** Active server name (determines which server's iframe to show) */
  activeServer?: string
  className?: string
}

export type ScriptHealthStatus = 'not-injected' | 'injected-not-reachable' | 'injected-reachable'
