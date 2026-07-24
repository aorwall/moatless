import { unwrapResponse } from '@moatless/api-client/custom-instance'
import type {
  GitHubAppsResponse,
  GitHubInstallationBindingRequest,
  GitHubInstallationBindingResponse,
} from '@moatless/api-client/sandbox/model'
import {
  adminDeleteGlobalGithubInstallation,
  adminDeleteTeamGithubInstallation,
  adminGetGlobalGithubInstallation,
  adminGetTeamGithubInstallation,
  adminListGithubApps,
  adminPutGlobalGithubInstallation,
  adminPutTeamGithubInstallation,
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

  getTeamInstallation: async (teamId: string): Promise<GitHubInstallationBindingResponse> =>
    await unwrapResponse<GitHubInstallationBindingResponse>(adminGetTeamGithubInstallation(teamId)),

  saveTeamInstallation: async (
    teamId: string,
    request: GitHubInstallationBindingRequest,
  ): Promise<GitHubInstallationBindingResponse> =>
    await unwrapResponse<GitHubInstallationBindingResponse>(
      adminPutTeamGithubInstallation(teamId, request),
    ),

  removeTeamInstallation: async (teamId: string): Promise<GitHubInstallationBindingResponse> =>
    await unwrapResponse<GitHubInstallationBindingResponse>(
      adminDeleteTeamGithubInstallation(teamId),
    ),
}
