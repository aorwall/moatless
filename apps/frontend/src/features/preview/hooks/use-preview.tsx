import { createContext, type ReactNode, useContext } from 'react'
import { usePreviewStore } from '../stores/preview-store'
import type { ElementInfo, PreviewConfig } from '../types'

interface PreviewContextValue {
  config: PreviewConfig
  currentPath: string
  isInspectorActive: boolean
  isLoaded: boolean
  selectedElement: ElementInfo | null
  error: string | null
  isConnected: boolean
  setCurrentPath: (path: string) => void
  toggleInspector: () => void
}

const PreviewContext = createContext<PreviewContextValue | null>(null)

export function PreviewProvider({ children }: { children: ReactNode }) {
  const store = usePreviewStore()

  const value: PreviewContextValue = {
    config: store.config,
    currentPath: store.currentPath,
    isInspectorActive: store.isInspectorActive,
    isLoaded: store.isLoaded,
    selectedElement: store.selectedElement,
    error: store.error,
    isConnected: store.isConnected,
    setCurrentPath: store.setCurrentPath,
    toggleInspector: store.toggleInspector,
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
