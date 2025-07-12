import type {
  BranchListResponse,
  CreateRepositoryRequest,
  FileSearchRequest,
  FileSearchResponse,
  RepositoryList,
  RepositoryResponse,
  UpdateRepositoryRequest,
} from '@moatless/shared/types/repository'
import { apiClient } from '@/lib/api/client'

export const repositoriesApi = {
  async listRepositories(): Promise<Array<RepositoryResponse>> {
    const response = await apiClient.get<RepositoryList>('/api/v1/repositories')
    return response.repositories
  },

  async getRepository(repoId: string): Promise<RepositoryResponse> {
    return await apiClient.get<RepositoryResponse>(`/api/v1/repositories/${repoId}`)
  },

  async createRepository(request: CreateRepositoryRequest): Promise<RepositoryResponse> {
    return await apiClient.post<RepositoryResponse>('/api/v1/repositories', request)
  },

  async updateRepository(
    repoId: string,
    request: UpdateRepositoryRequest,
  ): Promise<RepositoryResponse> {
    return await apiClient.put<RepositoryResponse>(`/api/v1/repositories/${repoId}`, request)
  },

  async deleteRepository(repoId: string): Promise<void> {
    await apiClient.delete(`/api/v1/repositories/${repoId}`)
  },

  async searchFiles(repoId: string, request: FileSearchRequest): Promise<FileSearchResponse> {
    return await apiClient.post<FileSearchResponse>(
      `/api/v1/repositories/${repoId}/search-files`,
      request,
    )
  },

  async getBranches(repoId: string): Promise<BranchListResponse> {
    return await apiClient.get<BranchListResponse>(`/api/v1/repositories/${repoId}/branches`)
  },
}
