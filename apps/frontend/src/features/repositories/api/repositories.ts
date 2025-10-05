import {
  buildPath,
  type RouteParams,
  type RouteRequestJson,
  type RouteSuccess,
} from '@moatless/api-contract'
import {
  createRepositoryRoute,
  deleteRepositoryRoute,
  getRepositoryBranchesRoute,
  getRepositoryRoute,
  listRepositoriesRoute,
  searchRepositoryFilesRoute,
  updateRepositoryRoute,
  verifyRepositoryRoute,
} from '@moatless/api-contract/routes/repositories'
import type {
  BranchListResponse,
  CommandExecutionOptions,
  FileSearchResponse,
  SetupScriptResponse,
} from '@moatless/types'
import { apiClient } from '@/lib/api/client'

type RepositoryListResponse = RouteSuccess<typeof listRepositoriesRoute>
type RepositoryResponse = RouteSuccess<typeof getRepositoryRoute>
type CreateRepositoryRequest = RouteRequestJson<typeof createRepositoryRoute>
type UpdateRepositoryRequest = RouteRequestJson<typeof updateRepositoryRoute>
type FileSearchRequest = RouteRequestJson<typeof searchRepositoryFilesRoute>

export const repositoriesApi = {
  async listRepositories(): Promise<Array<RepositoryResponse>> {
    const path = buildPath(listRepositoriesRoute)
    const response = await apiClient.get<RepositoryListResponse>(path)
    return response.repositories
  },

  async getRepository(
    repoId: RouteParams<typeof getRepositoryRoute>['repositoryId'],
  ): Promise<RepositoryResponse> {
    const path = buildPath(getRepositoryRoute, { repositoryId: repoId })
    return await apiClient.get<RepositoryResponse>(path)
  },

  async createRepository(request: CreateRepositoryRequest): Promise<RepositoryResponse> {
    const path = buildPath(createRepositoryRoute)
    return await apiClient.post<RepositoryResponse>(path, request)
  },

  async updateRepository(
    repoId: RouteParams<typeof updateRepositoryRoute>['repositoryId'],
    request: UpdateRepositoryRequest,
  ): Promise<RepositoryResponse> {
    const path = buildPath(updateRepositoryRoute, { repositoryId: repoId })
    return await apiClient.put<RepositoryResponse>(path, request)
  },

  async deleteRepository(
    repoId: RouteParams<typeof deleteRepositoryRoute>['repositoryId'],
  ): Promise<void> {
    const path = buildPath(deleteRepositoryRoute, { repositoryId: repoId })
    await apiClient.delete(path)
  },

  async searchFiles(
    repoId: RouteParams<typeof searchRepositoryFilesRoute>['repositoryId'],
    request: FileSearchRequest,
  ): Promise<FileSearchResponse> {
    const path = buildPath(searchRepositoryFilesRoute, { repositoryId: repoId })
    return await apiClient.post<FileSearchResponse>(path, request)
  },

  async getBranches(
    repoId: RouteParams<typeof getRepositoryBranchesRoute>['repositoryId'],
  ): Promise<BranchListResponse> {
    const path = buildPath(getRepositoryBranchesRoute, { repositoryId: repoId })
    return await apiClient.get<BranchListResponse>(path)
  },

  async runSetupScript(repoId: string): Promise<SetupScriptResponse> {
    // TODO: No route descriptor exists for this endpoint - needs backend migration
    return await apiClient.post<SetupScriptResponse>(`/api/v1/repositories/${repoId}/run-setup`)
  },

  async verifyRepository(
    repoId: RouteParams<typeof verifyRepositoryRoute>['repositoryId'],
  ): Promise<RouteSuccess<typeof verifyRepositoryRoute>> {
    const path = buildPath(verifyRepositoryRoute, { repositoryId: repoId })
    return await apiClient.post<RouteSuccess<typeof verifyRepositoryRoute>>(path)
  },

  async executeCommands(repoId: string, options: CommandExecutionOptions): Promise<Response> {
    // TODO: No route descriptor exists for this SSE endpoint - needs backend migration
    // For SSE, we need to handle the request differently - return the raw response
    // The hook will handle creating the EventSource connection
    return await fetch(`/api/v1/repositories/${repoId}/execute-commands`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify(options),
    })
  },
}
