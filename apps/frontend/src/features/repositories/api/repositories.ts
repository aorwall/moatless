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
import type {
  BranchListResponse,
  CreateRepositoryRequest,
  RepositoryResponse,
  UpdateRepositoryRequest,
} from '@moatless/types'

export const repositoriesApi = {
  async listRepositories(): Promise<RepositoryResponse[]> {
    const result = await unwrapResponse(listRepositoriesHandler())
    return result as unknown as RepositoryResponse[]
  },

  async getRepository(repoId: string): Promise<RepositoryResponse> {
    const result = await unwrapResponse(getRepositoryHandler(repoId))
    return result as unknown as RepositoryResponse
  },

  async createRepository(request: CreateRepositoryRequest): Promise<RepositoryResponse> {
    const result = await unwrapResponse(
      createRepositoryHandler(request as unknown as Parameters<typeof createRepositoryHandler>[0]),
    )
    return result as unknown as RepositoryResponse
  },

  async updateRepository(
    repoId: string,
    request: UpdateRepositoryRequest,
  ): Promise<RepositoryResponse> {
    const result = await unwrapResponse(
      updateRepositoryHandler(
        repoId,
        request as unknown as Parameters<typeof updateRepositoryHandler>[1],
      ),
    )
    return result as unknown as RepositoryResponse
  },

  async deleteRepository(repoId: string): Promise<void> {
    await unwrapResponse<void>(deleteRepositoryHandler(repoId))
  },

  async convertToTemplate(repoId: string): Promise<RepositoryResponse> {
    const result = await unwrapResponse(convertToTemplateHandler(repoId))
    return result as unknown as RepositoryResponse
  },

  async getBranches(repoId: string): Promise<BranchListResponse> {
    const result = await unwrapResponse(getBranchesHandler(repoId))
    return result as unknown as BranchListResponse
  },
}
