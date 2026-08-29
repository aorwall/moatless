import type { ListSkillsResponse, SkillResponse } from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { listRepositorySkills as listRepositorySkillsHandler } from '@moatless/api-client/sandbox/skills/skills'

export const skillsApi = {
  /**
   * The skills available in a repository. Pass the workspace a task runs in and
   * the list is narrowed to what that workspace actually loads.
   */
  async listRepositorySkills(repositoryId: string, workspaceId?: string): Promise<SkillResponse[]> {
    const response = await unwrapResponse<ListSkillsResponse>(
      listRepositorySkillsHandler(repositoryId, workspaceId === undefined ? {} : { workspaceId }),
    )
    return response.skills
  },
}
