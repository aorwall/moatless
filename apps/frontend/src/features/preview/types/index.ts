import type { ResourceMetrics, ServerEntry } from '@moatless/api-client'

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

export interface StartupEvent {
  reason: string
  message: string
  eventType: string
  timestamp: string
}

export interface SandboxEvent {
  eventId: string
  type: 'sandbox' | 'server'
  subType: string
  timestamp: string
  isError: boolean
  message?: string
  reason?: string
  /** Server name for server events (e.g., "preview", "files") */
  serverName?: string
  payload: Record<string, unknown>
}

export interface PreviewProps {
  taskId: string
  environmentStatus?:
    | 'not_created'
    | 'initializing'
    | 'ready'
    | 'stopped'
    | 'removing'
    | 'removed'
    | 'error'
  /** Error message when environment is in error state */
  environmentError?: string
  /** K8s startup events to display during initializing state */
  startupEvents?: StartupEvent[]
  onElementSelected?: (element: ElementInfo | null) => void
  onToggleElementContext?: (element: ElementInfo) => void
  onNavigationReady?: (navigate: (path: string) => void) => void
  onTroubleshootRequest?: (reason: string) => void
  /** Dynamic server list for multi-server tab rendering */
  servers?: ServerEntry[]
  /** Active server name (determines which server's iframe to show) */
  activeServer?: string
  /** Resource metrics from the sandbox container (CPU, memory, event loop) */
  resources?: ResourceMetrics
  /** Sandbox and server lifecycle events for the Events tab / recent events box */
  sandboxEvents?: SandboxEvent[]
  /** Hide the preview header (address bar, server tabs, inspector toggle, reload). */
  hideHeader?: boolean
  /** Hide the bottom logs/console panel entirely (no expanded panel, no collapsed bar). */
  hideConsole?: boolean
  className?: string
}

export type ScriptHealthStatus = 'not-injected' | 'injected-not-reachable' | 'injected-reachable'
