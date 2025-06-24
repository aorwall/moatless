import type {
  BranchListResponse,
  CreateEnvironmentRequest,
  EnvironmentList,
  EnvironmentResponse,
  FileSearchRequest,
  FileSearchResponse,
  SetupScriptResponse,
  UpdateEnvironmentRequest,
} from '@shared/types/environmenttypes/environment'
import { apiClient } from '@/lib/api/client'

export const environmentsApi = {
  async listEnvironments(): Promise<Array<EnvironmentResponse>> {
    const response = await apiClient.get<EnvironmentList>('/api/v1/environments')
    return response.environments
  },

  async getEnvironment(envId: string): Promise<EnvironmentResponse> {
    return await apiClient.get<EnvironmentResponse>(`/api/v1/environments/${envId}`)
  },

  async createEnvironment(request: CreateEnvironmentRequest): Promise<EnvironmentResponse> {
    return await apiClient.post<EnvironmentResponse>('/api/v1/environments', request)
  },

  async updateEnvironment(
    envId: string,
    request: UpdateEnvironmentRequest,
  ): Promise<EnvironmentResponse> {
    return await apiClient.put<EnvironmentResponse>(`/api/v1/environments/${envId}`, request)
  },

  async deleteEnvironment(envId: string): Promise<void> {
    await apiClient.delete(`/api/v1/environments/${envId}`)
  },

  async runSetupScript(envId: string): Promise<SetupScriptResponse> {
    return await apiClient.post<SetupScriptResponse>(`/api/v1/environments/${envId}/run-setup`)
  },

  async searchFiles(envId: string, request: FileSearchRequest): Promise<FileSearchResponse> {
    return await apiClient.post<FileSearchResponse>(
      `/api/v1/environments/${envId}/search-files`,
      request,
    )
  },

  async getBranches(envId: string): Promise<BranchListResponse> {
    return await apiClient.get<BranchListResponse>(`/api/v1/environments/${envId}/branches`)
  },
}
