import { useLocalStorage } from '@/lib/hooks/use-local-storage'

type ViewMode = 'code' | 'vibe'
type AgentMode = 'edit' | 'plan'

interface TaskPreferences {
  viewMode: ViewMode
  agentMode: AgentMode
  repositoryId: string
  templateId: string
  setViewMode: (mode: ViewMode) => void
  setAgentMode: (mode: AgentMode) => void
  setRepositoryId: (id: string) => void
  setTemplateId: (id: string) => void
}

/**
 * Hook to manage task creation preferences with localStorage persistence.
 *
 * Provides a clean API for getting and setting all task-related preferences:
 * - View mode (code/vibe)
 * - Agent mode (edit/plan)
 * - Repository ID
 * - Template ID
 *
 * All values are automatically persisted to localStorage and synchronized
 * across browser tabs/windows.
 *
 * Note: Model selection is not persisted - components should use DEFAULT_CLAUDE_MODEL
 */
export function useTaskPreferences(): TaskPreferences {
  const [viewMode, setViewMode] = useLocalStorage<ViewMode>('lastViewMode', 'code')
  const [agentMode, setAgentMode] = useLocalStorage<AgentMode>('lastSelectedAgentMode', 'edit')
  const [repositoryId, setRepositoryId] = useLocalStorage<string>('lastSelectedRepositoryId', '')
  const [templateId, setTemplateId] = useLocalStorage<string>('lastSelectedTemplateId', '')

  return {
    viewMode,
    agentMode,
    repositoryId,
    templateId,
    setViewMode,
    setAgentMode,
    setRepositoryId,
    setTemplateId,
  }
}
