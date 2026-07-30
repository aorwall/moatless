import { unwrapResponse } from '@moatless/api-client/custom-instance'
import type {
  CreateUserResponse,
  GitHubAppRegistrationResponse,
  GitHubAppsResponse,
  GitHubInstallationBindingRequest,
  GitHubInstallationBindingResponse,
  RegisterGitHubAppRequest,
  RemoveGitHubAppResponse,
  RotateGitHubAppKeyRequest,
} from '@moatless/api-client/sandbox/model'
import {
  adminDeleteGlobalGithubInstallation,
  adminGetGlobalGithubInstallation,
  adminListGithubApps,
  adminPutGlobalGithubInstallation,
  adminRegisterGithubApp,
  adminRemoveGithubApp,
  adminRotateGithubAppKey,
} from '@moatless/api-client/sandbox/provider-settings/provider-settings'
import { createUserHandler } from '@moatless/api-client/sandbox/users/users'

export const adminGithubConnectionApi = {
  getApps: async (): Promise<GitHubAppsResponse> =>
    await unwrapResponse<GitHubAppsResponse>(adminListGithubApps()),

  registerApp: async (request: RegisterGitHubAppRequest): Promise<GitHubAppRegistrationResponse> =>
    await unwrapResponse<GitHubAppRegistrationResponse>(adminRegisterGithubApp(request)),

  rotateAppKey: async (
    githubAppKey: string,
    request: RotateGitHubAppKeyRequest,
  ): Promise<GitHubAppRegistrationResponse> =>
    await unwrapResponse<GitHubAppRegistrationResponse>(
      adminRotateGithubAppKey(githubAppKey, request),
    ),

  removeApp: async (githubAppKey: string): Promise<RemoveGitHubAppResponse> =>
    await unwrapResponse<RemoveGitHubAppResponse>(adminRemoveGithubApp(githubAppKey)),

  // Creating the Bot user is its own act: an App that only reaches
  // repositories never runs as anyone. Re-running it refreshes the identity
  // from GitHub rather than producing a second user.
  createBotUser: async (githubAppKey: string): Promise<CreateUserResponse> =>
    await unwrapResponse<CreateUserResponse>(createUserHandler({ isBot: true, githubAppKey })),

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
