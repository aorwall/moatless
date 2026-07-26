import type {
  AddRepoRequest,
  CreateWorkspaceRequest,
  ResetGitWorkspaceResponse,
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
  overrideWorkspace as overrideWorkspaceHandler,
  removeWorkspaceRepo as removeWorkspaceRepoHandler,
  resetGitWorkspace as resetGitWorkspaceHandler,
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

  /** Take a git-synced workspace under manual control so it can be edited. */
  async overrideWorkspace(
    workspaceId: string,
    request: UpdateWorkspaceRequest,
  ): Promise<WorkspaceResponse> {
    return await unwrapResponse(overrideWorkspaceHandler(workspaceId, request))
  },

  /** Discard the local override and re-sync the workspace from its repository. */
  async resetGitWorkspace(workspaceId: string): Promise<ResetGitWorkspaceResponse> {
    return await unwrapResponse(resetGitWorkspaceHandler(workspaceId))
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
