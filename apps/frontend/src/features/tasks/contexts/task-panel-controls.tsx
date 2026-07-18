import { createContext, type ReactNode, useContext, useMemo, useRef } from 'react'
import { type TaskView, useTaskView } from '../hooks/use-task-view'

/**
 * Semantic cross-panel actions, decoupled from any specific layout. Panels
 * trigger actions on sibling panels through this context; each layout registers
 * the layout-appropriate implementation (e.g. desktop wraps the resizable
 * panel's collapse, mobile maps `toggleChat` to a view switch).
 */
interface TaskPanelControls {
  /** Collapse/expand chat (desktop) or switch to the chat view (mobile). */
  toggleChat(): void
  /** Switch the workspace to a specific view (defaults to the current one). */
  showWorkspace(view?: TaskView): void
  /** Navigate the preview iframe to a path (backed by the registered navigator). */
  navigatePreview(path: string): void
  /** Reload the preview iframe (backed by the registered reloader; no-op if none). */
  reloadPreview(): void
  /** Invoke the command executor registered by the chat panel (no-op if none). */
  executeCommand(name: string): void
  /** Register the chat panel's command executor; returns an unregister fn. */
  registerCommandExecutor(fn: (name: string) => void): () => void
  /** Register the active layout's chat-toggle impl; returns an unregister fn. */
  registerChatToggle(fn: () => void): () => void
  /** Register the preview's imperative navigator; returns an unregister fn. */
  registerPreviewNavigate(fn: (path: string) => void): () => void
  /** Register the preview's imperative reloader; returns an unregister fn. */
  registerPreviewReload(fn: () => void): () => void
}

const TaskPanelControlsContext = createContext<TaskPanelControls | undefined>(undefined)

export function TaskPanelControlsProvider({ children }: { children: ReactNode }) {
  const { setView } = useTaskView()

  // Keep the latest setView without re-creating the (stable) context value.
  const setViewRef = useRef(setView)
  setViewRef.current = setView

  const chatToggleRef = useRef<(() => void) | null>(null)
  const commandExecutorRef = useRef<((name: string) => void) | null>(null)
  const previewNavigateRef = useRef<((path: string) => void) | null>(null)
  const previewReloadRef = useRef<(() => void) | null>(null)

  const value = useMemo<TaskPanelControls>(
    () => ({
      toggleChat: () => chatToggleRef.current?.(),
      showWorkspace: (view) => {
        if (view) {
          setViewRef.current(view)
        }
      },
      navigatePreview: (path) => previewNavigateRef.current?.(path),
      reloadPreview: () => previewReloadRef.current?.(),
      executeCommand: (name) => commandExecutorRef.current?.(name),
      registerCommandExecutor: (fn) => {
        commandExecutorRef.current = fn
        return () => {
          if (commandExecutorRef.current === fn) {
            commandExecutorRef.current = null
          }
        }
      },
      registerChatToggle: (fn) => {
        chatToggleRef.current = fn
        return () => {
          if (chatToggleRef.current === fn) {
            chatToggleRef.current = null
          }
        }
      },
      registerPreviewNavigate: (fn) => {
        previewNavigateRef.current = fn
        return () => {
          if (previewNavigateRef.current === fn) {
            previewNavigateRef.current = null
          }
        }
      },
      registerPreviewReload: (fn) => {
        previewReloadRef.current = fn
        return () => {
          if (previewReloadRef.current === fn) {
            previewReloadRef.current = null
          }
        }
      },
    }),
    [],
  )

  return <TaskPanelControlsContext value={value}>{children}</TaskPanelControlsContext>
}

export function usePanelControls(): TaskPanelControls {
  const context = useContext(TaskPanelControlsContext)
  if (!context) {
    throw new Error('usePanelControls must be used within TaskPanelControlsProvider')
  }
  return context
}
