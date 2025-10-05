import {
  buildPath,
  type RouteParams,
  type RouteRequestJson,
  type RouteSuccess,
} from '@moatless/api-contract'
import {
  getWorkspaceGitDiffRoute,
  getWorkspaceStatusRoute,
  searchWorkspaceFilesRoute,
  setupWorkspaceRoute,
} from '@moatless/api-contract/routes/workspace'
import type {
  CreateTaskRequest,
  DeployResponse,
  MergeTaskResponse,
  Message,
  MessagesResponse,
  SimpleTaskResponse,
  StopAgentResponse,
  Task,
  TaskCommandsResponse,
  TaskListResponse,
} from '@moatless/types'
import { apiClient } from '@/lib/api/client'

type WorkspaceStatusResponse = RouteSuccess<typeof getWorkspaceStatusRoute>
type WorkspaceSetupResponse = RouteSuccess<typeof setupWorkspaceRoute>
type FileSearchRequest = RouteRequestJson<typeof searchWorkspaceFilesRoute>
type FileSearchResponse = RouteSuccess<typeof searchWorkspaceFilesRoute>
type GitDiffResponse = RouteSuccess<typeof getWorkspaceGitDiffRoute>

export const tasksApi = {
  async getActiveTasks(): Promise<Array<string>> {
    const response = await apiClient.get<TaskListResponse>('/api/v1/tasks', {
      status: 'active',
    })
    return response.items.map((task) => task.taskId)
  },

  async listTasks(params?: {
    activeOnly?: boolean
    status?: 'open' | 'running' | 'error' | 'merged' | 'closed'
    limit?: number
    offset?: number
  }): Promise<TaskListResponse> {
    const response = await apiClient.get<TaskListResponse>('/api/v1/tasks', {
      status: params?.status || (params?.activeOnly ? 'active' : undefined),
      limit: params?.limit,
      offset: params?.offset,
    })
    return response
  },

  async getTask(taskId: string): Promise<Task> {
    return await apiClient.get<Task>(`/api/v1/tasks/${taskId}`)
  },

  async getTaskMetadata(taskId: string): Promise<Task> {
    return await apiClient.get<Task>(`/api/v1/tasks/${taskId}`)
  },

  async createTask(request: CreateTaskRequest): Promise<SimpleTaskResponse> {
    return await apiClient.post<SimpleTaskResponse>('/api/v1/tasks', request)
  },

  async sendMessage(
    taskId: string,
    message: string,
    resume?: boolean,
    model?: string,
    fileIds?: string[],
    filePaths?: string[],
    commandName?: string,
    agentMode?: string,
  ): Promise<SimpleTaskResponse> {
    return await apiClient.post<SimpleTaskResponse>(`/api/v1/tasks/${taskId}/messages`, {
      message,
      resume,
      model,
      fileIds,
      filePaths,
      commandName,
      agentMode,
    })
  },

  // Deprecated: Use createTask instead
  triggerAgent(request: CreateTaskRequest): Promise<SimpleTaskResponse> {
    return this.createTask(request)
  },

  // Deprecated: Use deleteTask instead
  async stopAgent(taskId: string): Promise<StopAgentResponse> {
    return await apiClient.post<StopAgentResponse>(`/api/v1/tasks/${taskId}/stop`)
  },

  async getTaskMessages(taskId: string): Promise<Array<Message>> {
    const response = await apiClient.get<MessagesResponse>(`/api/v1/tasks/${taskId}/messages`)
    return response.messages
  },

  async getTaskGitDiff(
    taskId: RouteParams<typeof getWorkspaceGitDiffRoute>['taskId'],
  ): Promise<GitDiffResponse> {
    const path = buildPath(getWorkspaceGitDiffRoute, { taskId })
    return await apiClient.get<GitDiffResponse>(path)
  },

  async searchTaskFiles(
    taskId: RouteParams<typeof searchWorkspaceFilesRoute>['taskId'],
    request: FileSearchRequest,
  ): Promise<FileSearchResponse> {
    const path = buildPath(searchWorkspaceFilesRoute, { taskId })
    return await apiClient.post<FileSearchResponse>(path, request)
  },

  async getTaskWorkspaceStatus(
    taskId: RouteParams<typeof getWorkspaceStatusRoute>['taskId'],
  ): Promise<WorkspaceStatusResponse> {
    const path = buildPath(getWorkspaceStatusRoute, { taskId })
    return await apiClient.get<WorkspaceStatusResponse>(path)
  },

  async setupWorkspace(
    taskId: RouteParams<typeof setupWorkspaceRoute>['taskId'],
  ): Promise<WorkspaceSetupResponse> {
    const path = buildPath(setupWorkspaceRoute, { taskId })
    return await apiClient.post<WorkspaceSetupResponse>(path, {})
  },

  async getTaskCommands(taskId: string): Promise<TaskCommandsResponse> {
    return await apiClient.get<TaskCommandsResponse>(`/api/v1/tasks/${taskId}/commands`)
  },

  async mergeTask(
    taskId: string,
    options?: { targetBranch?: string; commitMessage?: string },
  ): Promise<MergeTaskResponse> {
    return await apiClient.post<MergeTaskResponse>(`/api/v1/tasks/${taskId}/merge`, options || {})
  },

  async closeTask(taskId: string): Promise<{ status: 'closed'; taskId: string }> {
    return await apiClient.post<{ status: 'closed'; taskId: string }>(
      `/api/v1/tasks/${taskId}/close`,
    )
  },

  async deleteTask(taskId: string): Promise<{ status: string }> {
    return await apiClient.delete<{ status: string }>(`/api/v1/tasks/${taskId}`)
  },

  async deployTask(taskId: string): Promise<DeployResponse> {
    return await apiClient.post<DeployResponse>(`/api/v1/tasks/${taskId}/deploy`)
  },
}
