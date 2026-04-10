import type { ListSkillsResponse, SkillResponse } from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { listRepositorySkills as listRepositorySkillsHandler } from '@moatless/api-client/sandbox/skills/skills'

export const skillsApi = {
  async listRepositorySkills(repositoryId: string): Promise<SkillResponse[]> {
    const response = await unwrapResponse<ListSkillsResponse>(
      listRepositorySkillsHandler(repositoryId),
    )
    return response.skills
  },
}
