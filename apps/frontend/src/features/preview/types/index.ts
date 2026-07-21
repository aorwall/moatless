import type { ResourceMetrics, ServerEntry } from '@moatless/api-client'
import type { LifecycleEvent, StartupEvent } from '@/features/events/types'

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

/**
 * Publish (deploy) state supplied by the composing feature. Kept as an injected
 * prop so `features/preview` doesn't reach into task/repository data.
 */
export interface PreviewPublish {
  canPublish: boolean
  siteUrl?: string | undefined
  onPublish: () => void
  isPending: boolean
}

export interface PreviewProps {
  taskId: string
  sandboxStatus?:
    | 'not_created'
    | 'initializing'
    | 'ready'
    | 'stopped'
    | 'removing'
    | 'removed'
    | 'error'
    | undefined
  /** Error message when environment is in error state */
  sandboxError?: string | undefined
  /** K8s startup events to display during initializing state */
  startupEvents?: StartupEvent[] | undefined
  onElementSelected?: ((element: ElementInfo | null) => void) | undefined
  onToggleElementContext?: ((element: ElementInfo) => void) | undefined
  onNavigationReady?: ((navigate: (path: string) => void) => void) | undefined
  /** Register the preview's imperative reload fn with the composing feature. */
  onReloadReady?: ((reload: () => void) => void) | undefined
  onTroubleshootRequest?: ((reason: string) => void) | undefined
  /** Called when the preview iframe requests toggling the chat panel. */
  onChatToggleRequest?: (() => void) | undefined
  /** Dynamic server list for multi-server tab rendering */
  servers?: ServerEntry[] | undefined
  /** Active server name (determines which server's iframe to show) */
  activeServer?: string | undefined
  /** Active workspace view — highlights the matching header tab */
  view: string
  /** Switch the active workspace view (from the header tabs) */
  onViewChange: (view: string) => void
  /** Publish/deploy state for the header's publish button (undefined = no publish) */
  publish?: PreviewPublish | undefined
  /** Resource metrics from the sandbox container (CPU, memory, event loop) */
  resources?: ResourceMetrics | undefined
  /** Sandbox and server lifecycle events for the Events tab / recent events box */
  sandboxEvents?: LifecycleEvent[] | undefined
  /** Hide the preview header (address bar, server tabs, inspector toggle, reload). */
  hideHeader?: boolean | undefined
  /** Hide the bottom logs/console panel entirely (no expanded panel, no collapsed bar). */
  hideConsole?: boolean | undefined
  className?: string | undefined
}

export type ScriptHealthStatus = 'not-injected' | 'injected-not-reachable' | 'injected-reachable'
