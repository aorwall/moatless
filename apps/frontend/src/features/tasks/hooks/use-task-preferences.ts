import type { ClaudeModel } from '@moatless/types'
import { DEFAULT_CLAUDE_MODEL } from '@moatless/types'
import { useLocalStorage } from '@/lib/hooks/use-local-storage'

type ViewMode = 'code' | 'vibe'
type AgentMode = 'edit' | 'plan'

interface TaskPreferences {
  viewMode: ViewMode
  model: ClaudeModel
  agentMode: AgentMode
  repositoryId: string
  templateId: string
  setViewMode: (mode: ViewMode) => void
  setModel: (model: ClaudeModel) => void
  setAgentMode: (mode: AgentMode) => void
  setRepositoryId: (id: string) => void
  setTemplateId: (id: string) => void
}

/**
 * Hook to manage task creation preferences with localStorage persistence.
 *
 * Provides a clean API for getting and setting all task-related preferences:
 * - View mode (code/vibe)
 * - Selected Claude model
 * - Agent mode (edit/plan)
 * - Repository ID
 * - Template ID
 *
 * All values are automatically persisted to localStorage and synchronized
 * across browser tabs/windows.
 */
export function useTaskPreferences(): TaskPreferences {
  const [viewMode, setViewMode] = useLocalStorage<ViewMode>('lastViewMode', 'code')
  const [model, setModel] = useLocalStorage<ClaudeModel>('lastSelectedModel', DEFAULT_CLAUDE_MODEL)
  const [agentMode, setAgentMode] = useLocalStorage<AgentMode>('lastSelectedAgentMode', 'edit')
  const [repositoryId, setRepositoryId] = useLocalStorage<string>('lastSelectedRepositoryId', '')
  const [templateId, setTemplateId] = useLocalStorage<string>('lastSelectedTemplateId', '')

  return {
    viewMode,
    model,
    agentMode,
    repositoryId,
    templateId,
    setViewMode,
    setModel,
    setAgentMode,
    setRepositoryId,
    setTemplateId,
  }
}
