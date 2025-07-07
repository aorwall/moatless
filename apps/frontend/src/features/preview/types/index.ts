export interface ElementInfo {
  tagName: string
  className: string
  id: string
  textContent: string
  selector: string
  displayText: string
  elementPath: string[]
  attributes: Record<string, string>
  computedStyles: {
    display: string
    position: string
    width: string
    height: string
    color: string
    backgroundColor: string
    fontSize: string
    fontFamily: string
    padding: string
    margin: string
    border: string
  }
  boundingRect: {
    top: number
    left: number
    width: number
    height: number
    x: number
    y: number
  }
}

export interface PreviewConfig {
  url: string
  path?: string
  port?: number
}

export interface PreviewProps {
  config: PreviewConfig
  serverStatus?: {
    running: boolean
    configured: boolean
    url?: string
  }
  onElementSelected?: (element: ElementInfo | null) => void
  className?: string
}

export type ScriptHealthStatus = 'not-injected' | 'injected-not-reachable' | 'injected-reachable'

export interface PreviewState {
  isInspectorMode: boolean
  currentPath: string
  isLoaded: boolean
  error: string | null
  selectedElement: ElementInfo | null
  isConnected: boolean
  scriptHealthStatus: ScriptHealthStatus
  lastHeartbeat: number | null
}
