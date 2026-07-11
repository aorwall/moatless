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
  type: 'sandbox' | 'server' | 'taskTurn'
  subType: string
  timestamp: string
  isError: boolean
  message?: string | undefined
  reason?: string | undefined
  /** Server name for server events (e.g., "preview", "files") */
  serverName?: string | undefined
  /** Node the pod was scheduled on — enables node-level correlation of stops. */
  nodeName?: string | undefined
  /** Why the pod was disrupted (DisruptionTarget reason / pod status reason). */
  disruptionReason?: string | undefined
  /** Who caused an intentional stop ("reaper" | "user" | "eviction" | …). */
  initiator?: string | undefined
  /** Exit code of the terminated sandbox container, when known. */
  exitCode?: number | undefined
  /** K8s termination reason of the sandbox container, when known. */
  terminatedReason?: string | undefined
  /** Seconds spent Pending before the pod was scheduled (on pod_scheduled). */
  pendingSeconds?: number | undefined
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
    | undefined
  /** Error message when environment is in error state */
  environmentError?: string | undefined
  /** K8s startup events to display during initializing state */
  startupEvents?: StartupEvent[] | undefined
  onElementSelected?: ((element: ElementInfo | null) => void) | undefined
  onToggleElementContext?: ((element: ElementInfo) => void) | undefined
  onNavigationReady?: ((navigate: (path: string) => void) => void) | undefined
  onTroubleshootRequest?: ((reason: string) => void) | undefined
  /** Called when the preview iframe requests toggling the chat panel. */
  onChatToggleRequest?: (() => void) | undefined
  /** Dynamic server list for multi-server tab rendering */
  servers?: ServerEntry[] | undefined
  /** Active server name (determines which server's iframe to show) */
  activeServer?: string | undefined
  /** Resource metrics from the sandbox container (CPU, memory, event loop) */
  resources?: ResourceMetrics | undefined
  /** Sandbox and server lifecycle events for the Events tab / recent events box */
  sandboxEvents?: SandboxEvent[] | undefined
  /** Hide the preview header (address bar, server tabs, inspector toggle, reload). */
  hideHeader?: boolean | undefined
  /** Hide the bottom logs/console panel entirely (no expanded panel, no collapsed bar). */
  hideConsole?: boolean | undefined
  className?: string | undefined
}

export type ScriptHealthStatus = 'not-injected' | 'injected-not-reachable' | 'injected-reachable'
