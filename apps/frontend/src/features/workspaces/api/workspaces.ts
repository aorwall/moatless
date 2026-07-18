import type {
  AddRepoRequest,
  CreateWorkspaceRequest,
  UpdateWorkspaceRequest,
  WorkspaceResponse,
} from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  addWorkspaceRepo as addWorkspaceRepoHandler,
  createWorkspace as createWorkspaceHandler,
  deleteWorkspace as deleteWorkspaceHandler,
  getWorkspace as getWorkspaceHandler,
  listWorkspaces as listWorkspacesHandler,
  removeWorkspaceRepo as removeWorkspaceRepoHandler,
  setWorkspacePrimaryRepo as setWorkspacePrimaryRepoHandler,
  updateWorkspace as updateWorkspaceHandler,
} from '@moatless/api-client/sandbox/workspaces/workspaces'

export const workspacesApi = {
  async listWorkspaces(): Promise<WorkspaceResponse[]> {
    return await unwrapResponse(listWorkspacesHandler())
  },

  async getWorkspace(workspaceId: string): Promise<WorkspaceResponse> {
    return await unwrapResponse(getWorkspaceHandler(workspaceId))
  },

  async createWorkspace(request: CreateWorkspaceRequest): Promise<WorkspaceResponse> {
    return await unwrapResponse(createWorkspaceHandler(request))
  },

  async updateWorkspace(
    workspaceId: string,
    request: UpdateWorkspaceRequest,
  ): Promise<WorkspaceResponse> {
    return await unwrapResponse(updateWorkspaceHandler(workspaceId, request))
  },

  async deleteWorkspace(workspaceId: string): Promise<void> {
    await unwrapResponse<void>(deleteWorkspaceHandler(workspaceId))
  },

  async addRepo(workspaceId: string, request: AddRepoRequest): Promise<WorkspaceResponse> {
    return await unwrapResponse(addWorkspaceRepoHandler(workspaceId, request))
  },

  async removeRepo(workspaceId: string, placementId: string): Promise<WorkspaceResponse> {
    return await unwrapResponse(removeWorkspaceRepoHandler(workspaceId, placementId))
  },

  async setPrimaryRepo(workspaceId: string, placementId: string): Promise<WorkspaceResponse> {
    return await unwrapResponse(setWorkspacePrimaryRepoHandler(workspaceId, placementId))
  },
}
