import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  adminListGithubApps,
  adminRegisterGithubApp,
  adminRemoveGithubApp,
  adminRotateGithubAppKey,
} from '@moatless/api-client/sandbox/app-administration/app-administration'
import type {
  CreateUserResponse,
  GitHubAppRegistrationResponse,
  GitHubAppsResponse,
  RegisterGitHubAppRequest,
  RemoveGitHubAppResponse,
  RotateGitHubAppKeyRequest,
} from '@moatless/api-client/sandbox/model'
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
}
