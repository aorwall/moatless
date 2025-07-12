import { createContext, type ReactNode, useContext, useReducer } from 'react'
import type { ElementInfo, PreviewConfig, ScriptHealthStatus } from '../types'

interface PreviewState {
  // Configuration
  config: PreviewConfig
  currentPath: string // Path for iframe navigation (user-initiated changes only)
  displayPath: string // Path shown in address bar (updated by route detection)

  // UI State
  isInspectorActive: boolean
  isLoaded: boolean

  // Data
  selectedElement: ElementInfo | null
  error: string | null
  isConnected: boolean

  // Script Health
  scriptHealthStatus: ScriptHealthStatus
  lastHeartbeat: number | null
}

type PreviewAction =
  | { type: 'SET_CONFIG'; payload: PreviewConfig }
  | { type: 'SET_CURRENT_PATH'; payload: string }
  | { type: 'SET_DISPLAY_PATH'; payload: string }
  | { type: 'NAVIGATE_TO_PATH'; payload: string }
  | { type: 'TOGGLE_INSPECTOR' }
  | { type: 'SET_SELECTED_ELEMENT'; payload: ElementInfo | null }
  | { type: 'SET_ERROR'; payload: string | null }
  | { type: 'SET_CONNECTED'; payload: boolean }
  | { type: 'SET_LOADED'; payload: boolean }
  | { type: 'SET_SCRIPT_HEALTH_STATUS'; payload: ScriptHealthStatus }
  | { type: 'UPDATE_HEARTBEAT' }
  | { type: 'RESET' }

interface PreviewContextValue extends PreviewState {
  setConfig: (config: PreviewConfig) => void
  setCurrentPath: (path: string) => void // For user navigation - updates iframe
  setDisplayPath: (path: string) => void // For route detection - only updates display
  navigateToPath: (path: string) => void // For user navigation - updates both
  toggleInspector: () => void
  setSelectedElement: (element: ElementInfo | null) => void
  setError: (error: string | null) => void
  setConnected: (connected: boolean) => void
  setLoaded: (loaded: boolean) => void
  setScriptHealthStatus: (status: ScriptHealthStatus) => void
  updateHeartbeat: () => void
  reset: () => void
}

const initialState: PreviewState = {
  config: { url: 'http://localhost:3000' },
  currentPath: '/',
  displayPath: '/',
  isInspectorActive: false,
  isLoaded: false,
  selectedElement: null,
  error: null,
  isConnected: false,
  scriptHealthStatus: 'not-injected',
  lastHeartbeat: null,
}

function previewReducer(state: PreviewState, action: PreviewAction): PreviewState {
  switch (action.type) {
    case 'SET_CONFIG':
      return { ...state, config: action.payload }

    case 'SET_CURRENT_PATH': {
      const normalizedPath = action.payload.startsWith('/') ? action.payload : `/${action.payload}`
      return { ...state, currentPath: normalizedPath }
    }

    case 'SET_DISPLAY_PATH': {
      const normalizedPath = action.payload.startsWith('/') ? action.payload : `/${action.payload}`
      return { ...state, displayPath: normalizedPath }
    }

    case 'NAVIGATE_TO_PATH': {
      const normalizedPath = action.payload.startsWith('/') ? action.payload : `/${action.payload}`
      return { ...state, currentPath: normalizedPath, displayPath: normalizedPath }
    }

    case 'TOGGLE_INSPECTOR':
      return {
        ...state,
        isInspectorActive: !state.isInspectorActive,
        selectedElement: state.isInspectorActive ? null : state.selectedElement,
      }

    case 'SET_SELECTED_ELEMENT':
      return { ...state, selectedElement: action.payload }

    case 'SET_ERROR':
      return {
        ...state,
        error: action.payload,
        isLoaded: action.payload ? false : state.isLoaded,
      }

    case 'SET_CONNECTED':
      return { ...state, isConnected: action.payload }

    case 'SET_LOADED':
      return {
        ...state,
        isLoaded: action.payload,
        error: action.payload ? null : state.error,
      }

    case 'SET_SCRIPT_HEALTH_STATUS':
      return { ...state, scriptHealthStatus: action.payload }

    case 'UPDATE_HEARTBEAT':
      return {
        ...state,
        lastHeartbeat: Date.now(),
        scriptHealthStatus: 'injected-reachable',
      }

    case 'RESET':
      return { ...initialState }

    default:
      return state
  }
}

const PreviewContext = createContext<PreviewContextValue | null>(null)

export function PreviewProvider({ children }: { children: ReactNode }) {
  const [state, dispatch] = useReducer(previewReducer, initialState)

  const value: PreviewContextValue = {
    ...state,
    setConfig: (config) => dispatch({ type: 'SET_CONFIG', payload: config }),
    setCurrentPath: (path) => dispatch({ type: 'SET_CURRENT_PATH', payload: path }),
    setDisplayPath: (path) => dispatch({ type: 'SET_DISPLAY_PATH', payload: path }),
    navigateToPath: (path) => dispatch({ type: 'NAVIGATE_TO_PATH', payload: path }),
    toggleInspector: () => dispatch({ type: 'TOGGLE_INSPECTOR' }),
    setSelectedElement: (element) => dispatch({ type: 'SET_SELECTED_ELEMENT', payload: element }),
    setError: (error) => dispatch({ type: 'SET_ERROR', payload: error }),
    setConnected: (connected) => dispatch({ type: 'SET_CONNECTED', payload: connected }),
    setLoaded: (loaded) => dispatch({ type: 'SET_LOADED', payload: loaded }),
    setScriptHealthStatus: (status) =>
      dispatch({ type: 'SET_SCRIPT_HEALTH_STATUS', payload: status }),
    updateHeartbeat: () => dispatch({ type: 'UPDATE_HEARTBEAT' }),
    reset: () => dispatch({ type: 'RESET' }),
  }

  return <PreviewContext.Provider value={value}>{children}</PreviewContext.Provider>
}

export function usePreview() {
  const context = useContext(PreviewContext)
  if (!context) {
    throw new Error('usePreview must be used within PreviewProvider')
  }
  return context
}
