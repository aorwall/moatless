import { useAgents } from '@/features/agents/hooks/use-agents'
import type { AgentDescriptor } from '@/features/agents/types'
import { type ClaudeModel, DEFAULT_CLAUDE_MODEL } from '@/lib/types/claude-models'

interface UseAgentAvailabilityArgs {
  agentType: string
  model: ClaudeModel
}

interface UseAgentAvailabilityResult {
  agents: AgentDescriptor[] | undefined
  selectedAgent: AgentDescriptor | undefined
  agentUnavailable: boolean
  settingsSection: string
  availableModel: ClaudeModel
}

// Encapsulates agent catalog lookup, model reset on agent change, and the
// "blocked because no credential" check used by the composer.
export function useAgentAvailability({
  agentType,
  model,
}: UseAgentAvailabilityArgs): UseAgentAvailabilityResult {
  const { data: agents } = useAgents()

  const selectedAgent = agents?.find((a) => a.id === agentType)

  const availableModel =
    selectedAgent && !selectedAgent.models.some((candidate) => candidate.id === model)
      ? ((selectedAgent.defaultModel ?? DEFAULT_CLAUDE_MODEL) as ClaudeModel)
      : model

  // Only block once the catalog has loaded — undefined during fetch means
  // "unknown availability", which shouldn't flash a disabled state.
  const agentUnavailable = !!agents && !!selectedAgent && !selectedAgent.installed
  const settingsSection = agentType === 'codex' ? 'codex' : 'claude'

  return { agents, selectedAgent, agentUnavailable, settingsSection, availableModel }
}
