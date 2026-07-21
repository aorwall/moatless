import { createContext, type ReactNode, useCallback, useContext, useMemo, useRef } from 'react'
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
  repositoryId: string
  branch: string
  agentType: AgentType
  agentMode: 'edit' | 'plan'
  model: ClaudeModel
  templateId?: string | undefined
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
  const { repositoryId, agentMode, agentType } = useTaskPreferences()

  // Keep a ref to the latest windows so the callbacks below stay stable and
  // never operate on a stale snapshot when several updates fire in one tick
  // (e.g. rapid typing across windows).
  const windowsRef = useRef(windows)
  windowsRef.current = windows

  const open = useCallback(() => {
    const newWindow: ComposeWindowState = {
      id: crypto.randomUUID(),
      minimized: false,
      name: '',
      message: '',
      repositoryId,
      branch: '',
      agentType,
      agentMode,
      model: DEFAULT_CLAUDE_MODEL,
      templateId: undefined,
    }
    setWindows([...windowsRef.current, newWindow])
  }, [setWindows, repositoryId, agentMode, agentType])

  const close = useCallback(
    (id: string) => {
      setWindows(windowsRef.current.filter((w) => w.id !== id))
    },
    [setWindows],
  )

  const toggleMinimize = useCallback(
    (id: string) => {
      setWindows(
        windowsRef.current.map((w) => (w.id === id ? { ...w, minimized: !w.minimized } : w)),
      )
    },
    [setWindows],
  )

  const update = useCallback(
    (id: string, patch: Partial<ComposeWindowState>) => {
      setWindows(windowsRef.current.map((w) => (w.id === id ? { ...w, ...patch } : w)))
    },
    [setWindows],
  )

  const value = useMemo<ComposeTaskContextValue>(
    () => ({ windows, open, close, toggleMinimize, update }),
    [windows, open, close, toggleMinimize, update],
  )

  return <ComposeTaskContext value={value}>{children}</ComposeTaskContext>
}

export function useComposeTask(): ComposeTaskContextValue {
  const ctx = useContext(ComposeTaskContext)
  if (!ctx) {
    throw new Error('useComposeTask must be used within a ComposeTaskProvider')
  }
  return ctx
}
