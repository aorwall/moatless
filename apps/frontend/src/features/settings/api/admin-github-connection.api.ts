import { unwrapResponse } from '@moatless/api-client/custom-instance'
import type { GitHubConfigResponse, SavePatRequest } from '@moatless/api-client/sandbox/model'
import {
  adminDeleteGlobalGithubPat as adminDeleteGlobalGithubPatHandler,
  adminGetGlobalGithubConfig as adminGetGlobalGithubConfigHandler,
  adminSaveGlobalGithubPat as adminSaveGlobalGithubPatHandler,
} from '@moatless/api-client/sandbox/provider-settings/provider-settings'

/**
 * The org-wide GitHub token: one credential an admin sets once, used by any
 * user who has none of their own. Admin-only on the backend.
 */
export const adminGithubConnectionApi = {
  getConfig: async (): Promise<GitHubConfigResponse> => {
    return await unwrapResponse<GitHubConfigResponse>(adminGetGlobalGithubConfigHandler())
  },

  savePAT: async (request: SavePatRequest): Promise<GitHubConfigResponse> => {
    return await unwrapResponse<GitHubConfigResponse>(
      adminSaveGlobalGithubPatHandler({ token: request.token }),
    )
  },

  remove: async (): Promise<GitHubConfigResponse> => {
    return await unwrapResponse<GitHubConfigResponse>(adminDeleteGlobalGithubPatHandler())
  },
}
