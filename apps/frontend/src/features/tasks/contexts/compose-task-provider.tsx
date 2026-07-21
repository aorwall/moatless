import { createContext, type ReactNode, useContext } from 'react'
import type { AgentType } from '@/features/tasks/components/chat/chat-input/selectors'
import { useLocalStorage } from '@/lib/hooks/use-local-storage'
import { type ClaudeModel, DEFAULT_CLAUDE_MODEL } from '@/lib/types/claude-models'
import { useTaskPreferences } from '../hooks/use-task-preferences'

/**
 * A single Gmail-style compose window. Each open window keeps its own draft
 * (message + repo/branch/agent selections) so multiple tasks can be composed
 * at once. The full list is persisted to localStorage so drafts survive reloads.
 */
export interface ComposeWindowState {
  id: string
  minimized: boolean
  /** Optional task name (the "subject"). Left blank, the agent generates one. */
  name: string
  message: string
  workspaceId: string
  branch: string
  agentType: AgentType
  agentMode: 'edit' | 'plan'
  model: ClaudeModel
}

interface ComposeTaskContextValue {
  windows: ComposeWindowState[]
  /** Open a new compose window seeded from the user's last-used preferences. */
  open: () => void
  close: (id: string) => void
  toggleMinimize: (id: string) => void
  /** Patch a window's draft fields (message, selections, …). */
  update: (id: string, patch: Partial<ComposeWindowState>) => void
}

const ComposeTaskContext = createContext<ComposeTaskContextValue | null>(null)

const STORAGE_KEY = 'composeTaskWindows'
const DEFAULT_WINDOWS: ComposeWindowState[] = []

export function ComposeTaskProvider({ children }: { children: ReactNode }) {
  const [windows, setWindows] = useLocalStorage<ComposeWindowState[]>(STORAGE_KEY, DEFAULT_WINDOWS)
  const { workspaceId, agentMode, agentType } = useTaskPreferences()

  const open = () => {
    const newWindow: ComposeWindowState = {
      id: crypto.randomUUID(),
      minimized: false,
      name: '',
      message: '',
      workspaceId,
      branch: '',
      agentType,
      agentMode,
      model: DEFAULT_CLAUDE_MODEL,
    }
    setWindows((currentWindows) => [...currentWindows, newWindow])
  }

  const close = (id: string) => {
    setWindows((currentWindows) => currentWindows.filter((window) => window.id !== id))
  }

  const toggleMinimize = (id: string) => {
    setWindows((currentWindows) =>
      currentWindows.map((window) =>
        window.id === id ? { ...window, minimized: !window.minimized } : window,
      ),
    )
  }

  const update = (id: string, patch: Partial<ComposeWindowState>) => {
    setWindows((currentWindows) =>
      currentWindows.map((window) => (window.id === id ? { ...window, ...patch } : window)),
    )
  }

  const value = {
    windows,
    open,
    close,
    toggleMinimize,
    update,
  }

  return <ComposeTaskContext value={value}>{children}</ComposeTaskContext>
}

export function useComposeTask(): ComposeTaskContextValue {
  const ctx = useContext(ComposeTaskContext)
  if (!ctx) {
    throw new Error('useComposeTask must be used within a ComposeTaskProvider')
  }
  return ctx
}
