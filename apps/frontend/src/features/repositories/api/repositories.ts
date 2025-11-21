import {
  buildPath,
  type RouteParams,
  type RouteRequestJson,
  type RouteSuccess,
} from '@moatless/api-contract'
import {
  convertToTemplateRoute,
  createRemoteRepositoryFromLocalRoute,
  createRepositoryRoute,
  deleteRepositoryRoute,
  getRepositoryBranchesRoute,
  getRepositoryRoute,
  listRepositoriesRoute,
  updateRepositoryRoute,
} from '@moatless/api-contract/repositories.ts'
import type { BranchListResponse } from '@moatless/types'
import { apiClient } from '@/lib/api/client'

type RepositoryListResponse = RouteSuccess<typeof listRepositoriesRoute>
type RepositoryResponse = RouteSuccess<typeof getRepositoryRoute>
type CreateRepositoryRequest = RouteRequestJson<typeof createRepositoryRoute>
type UpdateRepositoryRequest = RouteRequestJson<typeof updateRepositoryRoute>
type CreateRemoteRepositoryFromLocalRequest = RouteRequestJson<
  typeof createRemoteRepositoryFromLocalRoute
>

export const repositoriesApi = {
  async listRepositories(): Promise<RepositoryResponse[]> {
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

  async convertToTemplate(
    repoId: RouteParams<typeof convertToTemplateRoute>['repositoryId'],
  ): Promise<RepositoryResponse> {
    const path = buildPath(convertToTemplateRoute, { repositoryId: repoId })
    return await apiClient.post<RepositoryResponse>(path)
  },

  async getBranches(
    repoId: RouteParams<typeof getRepositoryBranchesRoute>['repositoryId'],
  ): Promise<BranchListResponse> {
    const path = buildPath(getRepositoryBranchesRoute, {
      repositoryId: repoId,
    })
    return await apiClient.get<BranchListResponse>(path)
  },

  async createRemoteRepositoryFromLocal(
    repoId: RouteParams<typeof createRemoteRepositoryFromLocalRoute>['repositoryId'],
    request: CreateRemoteRepositoryFromLocalRequest,
  ): Promise<RepositoryResponse> {
    const path = buildPath(createRemoteRepositoryFromLocalRoute, { repositoryId: repoId })
    return await apiClient.post<RepositoryResponse>(path, request)
  },
}
