import { useQuery } from '@tanstack/react-query'
import { agentsApi } from '@/features/agents/api/agents.api'
import type { AgentDescriptor } from '@/features/agents/types'
import { queryKeys } from '@/lib/api/query-keys'

const FIVE_MINUTES = 5 * 60 * 1000

export function useAgents() {
  return useQuery<AgentDescriptor[]>({
    queryKey: queryKeys.agents.list(),
    queryFn: async () => {
      const response = await agentsApi.listAgents()
      return response.agents
    },
    staleTime: FIVE_MINUTES,
  })
}
