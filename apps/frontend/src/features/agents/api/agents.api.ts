import { customInstance } from '@moatless/api-client/custom-instance'
import type { ListAgentsResponse } from '@/features/agents/types'

// Inlined fetch until the generated client is regenerated from the updated OpenAPI spec.
export const agentsApi = {
  async listAgents(): Promise<ListAgentsResponse> {
    return await customInstance<ListAgentsResponse>('/api/v1/agents', {
      method: 'GET',
    })
  },
}
