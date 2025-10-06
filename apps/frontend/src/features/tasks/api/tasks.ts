import {
  buildPath,
  type RouteParams,
  type RouteRequestJson,
  type RouteSuccess,
} from '@moatless/api-contract'
import {
  closeTaskRoute,
  createTaskRoute,
  deleteTaskRoute,
  deployRoute,
  getCommandsRoute,
  getMessagesRoute,
  getTaskRoute,
  listTasksRoute,
  sendMessageRoute,
  stopAgentRoute,
} from '@moatless/api-contract/tasks.ts'
import {
  getWorkspaceGitDiffRoute,
  getWorkspaceStatusRoute,
  searchWorkspaceFilesRoute,
  setupWorkspaceRoute,
} from '@moatless/api-contract/workspace.ts'
import type { CreateTaskRequest, Message } from '@moatless/types'
import { apiClient } from '@/lib/api/client'

type TaskListResponse = RouteSuccess<typeof listTasksRoute>
type WorkspaceStatusResponse = RouteSuccess<typeof getWorkspaceStatusRoute>
type WorkspaceSetupResponse = RouteSuccess<typeof setupWorkspaceRoute>
type FileSearchRequest = RouteRequestJson<typeof searchWorkspaceFilesRoute>
type FileSearchResponse = RouteSuccess<typeof searchWorkspaceFilesRoute>
type GitDiffResponse = RouteSuccess<typeof getWorkspaceGitDiffRoute>
type SimpleTaskResponse = RouteSuccess<typeof createTaskRoute>

export const tasksApi = {
  async getActiveTasks(): Promise<Array<string>> {
    const path = buildPath(listTasksRoute)
    const response = await apiClient.get<TaskListResponse>(path, {
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
    const path = buildPath(listTasksRoute)
    return await apiClient.get<TaskListResponse>(path, {
      status: params?.status || (params?.activeOnly ? 'active' : undefined),
      limit: params?.limit,
      offset: params?.offset,
    })
  },

  async getTask(taskId: string): Promise<RouteSuccess<typeof getTaskRoute>> {
    const path = buildPath(getTaskRoute, { taskId })
    return await apiClient.get<RouteSuccess<typeof getTaskRoute>>(path)
  },

  async getTaskMetadata(taskId: string): Promise<RouteSuccess<typeof getTaskRoute>> {
    const path = buildPath(getTaskRoute, { taskId })
    return await apiClient.get<RouteSuccess<typeof getTaskRoute>>(path)
  },

  async createTask(request: RouteRequestJson<typeof createTaskRoute>): Promise<SimpleTaskResponse> {
    const path = buildPath(createTaskRoute)
    return await apiClient.post<SimpleTaskResponse>(path, request)
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
  ): Promise<RouteSuccess<typeof sendMessageRoute>> {
    const path = buildPath(sendMessageRoute, { taskId })
    return await apiClient.post<RouteSuccess<typeof sendMessageRoute>>(path, {
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
  async stopAgent(taskId: string): Promise<RouteSuccess<typeof stopAgentRoute>> {
    const path = buildPath(stopAgentRoute, { taskId })
    return await apiClient.post<RouteSuccess<typeof stopAgentRoute>>(path)
  },

  async getTaskMessages(taskId: string): Promise<Array<Message>> {
    const path = buildPath(getMessagesRoute, { taskId })
    const response = await apiClient.get<RouteSuccess<typeof getMessagesRoute>>(path)
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

  async getTaskCommands(taskId: string): Promise<RouteSuccess<typeof getCommandsRoute>> {
    const path = buildPath(getCommandsRoute, { taskId })
    return await apiClient.get<RouteSuccess<typeof getCommandsRoute>>(path)
  },

  async closeTask(taskId: string): Promise<RouteSuccess<typeof closeTaskRoute>> {
    const path = buildPath(closeTaskRoute, { taskId })
    return await apiClient.post<RouteSuccess<typeof closeTaskRoute>>(path)
  },

  async deleteTask(taskId: string): Promise<RouteSuccess<typeof deleteTaskRoute>> {
    const path = buildPath(deleteTaskRoute, { taskId })
    return await apiClient.delete<RouteSuccess<typeof deleteTaskRoute>>(path)
  },

  async deployTask(taskId: string): Promise<RouteSuccess<typeof deployRoute>> {
    const path = buildPath(deployRoute, { taskId })
    return await apiClient.post<RouteSuccess<typeof deployRoute>>(path)
  },
}
