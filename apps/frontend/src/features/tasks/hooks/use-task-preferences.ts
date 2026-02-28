import { useLocalStorage } from '@/lib/hooks/use-local-storage'

type AgentMode = 'edit' | 'plan'

interface TaskPreferences {
  agentMode: AgentMode
  repositoryId: string
  setAgentMode: (mode: AgentMode) => void
  setRepositoryId: (id: string) => void
}

/**
 * Hook to manage task creation preferences with localStorage persistence.
 *
 * Provides a clean API for getting and setting all task-related preferences:
 * - Agent mode (edit/plan)
 * - Repository ID
 *
 * All values are automatically persisted to localStorage and synchronized
 * across browser tabs/windows.
 *
 * Note: Model selection is not persisted - components should use DEFAULT_CLAUDE_MODEL
 */
export function useTaskPreferences(): TaskPreferences {
  const [agentMode, setAgentMode] = useLocalStorage<AgentMode>('lastSelectedAgentMode', 'edit')
  const [repositoryId, setRepositoryId] = useLocalStorage<string>('lastSelectedRepositoryId', '')

  return {
    agentMode,
    repositoryId,
    setAgentMode,
    setRepositoryId,
  }
}
