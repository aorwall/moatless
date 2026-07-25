import { unwrapResponse } from '@moatless/api-client/custom-instance'
import type {
  GitHubAppsResponse,
  GitHubInstallationBindingRequest,
  GitHubInstallationBindingResponse,
} from '@moatless/api-client/sandbox/model'
import {
  adminDeleteGlobalGithubInstallation,
  adminGetGlobalGithubInstallation,
  adminListGithubApps,
  adminPutGlobalGithubInstallation,
} from '@moatless/api-client/sandbox/provider-settings/provider-settings'

export const adminGithubConnectionApi = {
  getApps: async (): Promise<GitHubAppsResponse> =>
    await unwrapResponse<GitHubAppsResponse>(adminListGithubApps()),

  getGlobalInstallation: async (): Promise<GitHubInstallationBindingResponse> =>
    await unwrapResponse<GitHubInstallationBindingResponse>(adminGetGlobalGithubInstallation()),

  saveGlobalInstallation: async (
    request: GitHubInstallationBindingRequest,
  ): Promise<GitHubInstallationBindingResponse> =>
    await unwrapResponse<GitHubInstallationBindingResponse>(
      adminPutGlobalGithubInstallation(request),
    ),

  removeGlobalInstallation: async (): Promise<GitHubInstallationBindingResponse> =>
    await unwrapResponse<GitHubInstallationBindingResponse>(adminDeleteGlobalGithubInstallation()),
}
