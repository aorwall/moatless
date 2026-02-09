import type {
  BranchListResponse,
  CreateRepositoryRequest,
  RepositoryResponse,
  UpdateRepositoryRequest,
} from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  convertToTemplate as convertToTemplateHandler,
  createRepository as createRepositoryHandler,
  deleteRepository as deleteRepositoryHandler,
  getBranches as getBranchesHandler,
  getRepository as getRepositoryHandler,
  listRepositories as listRepositoriesHandler,
  updateRepository as updateRepositoryHandler,
} from '@moatless/api-client/sandbox/repositories/repositories'

export const repositoriesApi = {
  async listRepositories(): Promise<RepositoryResponse[]> {
    return await unwrapResponse(listRepositoriesHandler())
  },

  async getRepository(repoId: string): Promise<RepositoryResponse> {
    return await unwrapResponse(getRepositoryHandler(repoId))
  },

  async createRepository(request: CreateRepositoryRequest): Promise<RepositoryResponse> {
    return await unwrapResponse(createRepositoryHandler(request))
  },

  async updateRepository(
    repoId: string,
    request: UpdateRepositoryRequest,
  ): Promise<RepositoryResponse> {
    return await unwrapResponse(updateRepositoryHandler(repoId, request))
  },

  async deleteRepository(repoId: string): Promise<void> {
    await unwrapResponse<void>(deleteRepositoryHandler(repoId))
  },

  async convertToTemplate(repoId: string): Promise<RepositoryResponse> {
    return await unwrapResponse(convertToTemplateHandler(repoId))
  },

  async getBranches(repoId: string): Promise<BranchListResponse> {
    return await unwrapResponse(getBranchesHandler(repoId))
  },
}
