import type {
  CreateTaskRequest,
  EventListResponse,
  MergeTaskResponse,
  SimpleTaskResponse,
  StopAgentResponse,
  TaskListResponse,
  WorkspaceStatusResponse,
} from '@shared/types/api'
import type { Event } from '@shared/types/event'
import type { FileSearchRequest, FileSearchResponse } from '@shared/types/repository'
import type { GitDiffResponse, Task } from '@shared/types/task'
import { apiClient } from '@/lib/api/client'

export const tasksApi = {
  async getActiveTasks(): Promise<Array<string>> {
    const response = await apiClient.get<TaskListResponse>('/api/v1/tasks', {
      status: 'active',
    })
    return response.tasks.map((task) => task.taskId)
  },

  async listTasks(params?: {
    activeOnly?: boolean
    status?: 'open' | 'running' | 'error' | 'merged' | 'closed'
    limit?: number
    offset?: number
  }): Promise<Array<Task>> {
    const response = await apiClient.get<TaskListResponse>('/api/v1/tasks', {
      status: params?.status || (params?.activeOnly ? 'active' : undefined),
      limit: params?.limit,
      offset: params?.offset,
    })
    return response.tasks
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
  ): Promise<SimpleTaskResponse> {
    return await apiClient.post<SimpleTaskResponse>(`/api/v1/tasks/${taskId}/messages`, {
      message,
      resume,
      model,
      fileIds,
      filePaths,
      commandName,
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

  async getTaskEvents(taskId: string, type?: string): Promise<Array<Event>> {
    const response = await apiClient.get<EventListResponse>(
      `/api/v1/tasks/${taskId}/events`,
      type ? { type } : undefined,
    )
    return response.events
  },

  async getTaskGitDiff(taskId: string): Promise<GitDiffResponse> {
    return await apiClient.get<GitDiffResponse>(`/api/v1/tasks/${taskId}/git-diff`)
  },

  async searchTaskFiles(taskId: string, request: FileSearchRequest): Promise<FileSearchResponse> {
    return await apiClient.post<FileSearchResponse>(`/api/v1/tasks/${taskId}/search-files`, request)
  },

  async getTaskWorkspaceStatus(taskId: string): Promise<WorkspaceStatusResponse> {
    return await apiClient.get<WorkspaceStatusResponse>(`/api/v1/tasks/${taskId}/workspace-status`)
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
}
