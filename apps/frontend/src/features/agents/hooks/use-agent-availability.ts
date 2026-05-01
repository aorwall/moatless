import { useEffect, useMemo } from 'react'
import { useAgents } from '@/features/agents/hooks/use-agents'
import type { AgentDescriptor } from '@/features/agents/types'
import { type ClaudeModel, DEFAULT_CLAUDE_MODEL } from '@/lib/types/claude-models'

interface UseAgentAvailabilityArgs {
  agentType: string
  model: ClaudeModel
  onModelChange?: (model: ClaudeModel) => void
}

interface UseAgentAvailabilityResult {
  agents: AgentDescriptor[] | undefined
  selectedAgent: AgentDescriptor | undefined
  agentUnavailable: boolean
  settingsSection: string
}

// Encapsulates agent catalog lookup, model reset on agent change, and the
// "blocked because no credential" check used by the composer.
export function useAgentAvailability({
  agentType,
  model,
  onModelChange,
}: UseAgentAvailabilityArgs): UseAgentAvailabilityResult {
  const { data: agents } = useAgents()

  const selectedAgent = useMemo(() => agents?.find((a) => a.id === agentType), [agents, agentType])

  useEffect(() => {
    if (!(selectedAgent && onModelChange)) {
      return
    }
    const defaultModel = (selectedAgent.defaultModel ?? DEFAULT_CLAUDE_MODEL) as ClaudeModel
    if (!selectedAgent.models.some((m) => m.id === model)) {
      onModelChange(defaultModel)
    }
  }, [selectedAgent, model, onModelChange])

  // Only block once the catalog has loaded — undefined during fetch means
  // "unknown availability", which shouldn't flash a disabled state.
  const agentUnavailable = !!agents && !!selectedAgent && !selectedAgent.installed
  const settingsSection = agentType === 'codex' ? 'codex' : 'claude'

  return { agents, selectedAgent, agentUnavailable, settingsSection }
}
