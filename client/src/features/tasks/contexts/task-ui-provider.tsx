import type { TaskIteration } from '@shared/types/task'
import { createContext, type ReactNode, useContext, useMemo, useState } from 'react'

type PanelView = 'events' | 'diff' | 'files' | 'preview' | 'server'

interface TaskUIContextValue {
  // Panel and layout state
  rightPanelView: PanelView
  selectedIteration: TaskIteration | undefined
  panelSizes: {
    left: number
    right: number
  }

  // File viewer state
  selectedFile: string

  // Actions
  setRightPanelView: (view: PanelView) => void
  setSelectedIteration: (iteration?: TaskIteration) => void
  setPanelSizes: (sizes: { left: number; right: number }) => void
  setSelectedFile: (file: string) => void
}

const TaskUIContext = createContext<TaskUIContextValue | undefined>(undefined)

interface TaskUIProviderProps {
  children: ReactNode
}

/**
 * TaskUIProvider manages pure UI state with no server dependencies.
 * This provider never causes re-renders due to data changes.
 */
export function TaskUIProvider({ children }: TaskUIProviderProps) {
  // UI state - all local, no server data
  const [rightPanelView, setRightPanelView] = useState<PanelView>('events')
  const [selectedIteration, setSelectedIteration] = useState<TaskIteration>()
  const [selectedFile, setSelectedFile] = useState<string>('')
  const [panelSizes, setPanelSizes] = useState({
    left: 60,
    right: 40,
  })

  // Context value is stable since it only contains local state
  const contextValue: TaskUIContextValue = useMemo(
    () => ({
      rightPanelView,
      selectedIteration,
      panelSizes,
      selectedFile,
      setRightPanelView,
      setSelectedIteration,
      setPanelSizes,
      setSelectedFile,
    }),
    [rightPanelView, selectedIteration, panelSizes, selectedFile],
  )

  return <TaskUIContext.Provider value={contextValue}>{children}</TaskUIContext.Provider>
}

/**
 * Hook to access UI state.
 * This never re-renders due to server data changes.
 */
export function useTaskUI() {
  const context = useContext(TaskUIContext)
  if (!context) {
    throw new Error('useTaskUI must be used within TaskUIProvider')
  }
  return context
}

// Specialized hooks for specific UI state
export function usePanelView() {
  const { rightPanelView, setRightPanelView } = useTaskUI()
  return { rightPanelView, setRightPanelView }
}

export function useSelectedIteration() {
  const { selectedIteration, setSelectedIteration } = useTaskUI()
  return { selectedIteration, setSelectedIteration }
}

export function usePanelSizes() {
  const { panelSizes, setPanelSizes } = useTaskUI()
  return { panelSizes, setPanelSizes }
}

export function useSelectedFile() {
  const { selectedFile, setSelectedFile } = useTaskUI()
  return { selectedFile, setSelectedFile }
}
