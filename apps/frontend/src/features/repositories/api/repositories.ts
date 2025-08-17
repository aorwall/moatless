import type {
  BranchListResponse,
  CommandExecutionOptions,
  CreateRepositoryRequest,
  FileSearchRequest,
  FileSearchResponse,
  RepositoryList,
  RepositoryResponse,
  SetupScriptResponse,
  UpdateRepositoryRequest,
  VerificationResponse,
} from '@moatless/types'
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

  async runSetupScript(repoId: string): Promise<SetupScriptResponse> {
    return await apiClient.post<SetupScriptResponse>(`/api/v1/repositories/${repoId}/run-setup`)
  },

  async verifyRepository(repoId: string): Promise<VerificationResponse> {
    return await apiClient.post<VerificationResponse>(`/api/v1/repositories/${repoId}/verify`)
  },

  async executeCommands(repoId: string, options: CommandExecutionOptions): Promise<Response> {
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
