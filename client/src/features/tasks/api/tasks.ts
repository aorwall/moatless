import type {
  FileSearchRequest,
  FileSearchResponse,
} from '@shared/types/environmenttypes/environment'
import type { Event } from '@shared/types/event'
import type { GitDiffResponse, TaskResponse } from '@shared/types/task'
import type { ServerStatus } from '@shared/types/workspace'
import { apiClient } from '@/lib/api/client'
import type { AgentTriggerRequest, AgentTriggerResponse } from '@/types/agent'
import type { WorkspaceStatusResponse } from '@/types/task'

export const tasksApi = {
  async getActiveTasks(): Promise<Array<string>> {
    const tasks = await apiClient.get<Array<TaskResponse>>('/api/v1/tasks', {
      active_only: true,
    })
    return tasks.map((task) => task.taskId)
  },

  async listTasks(params?: {
    activeOnly?: boolean
    limit?: number
    offset?: number
  }): Promise<Array<TaskResponse>> {
    return await apiClient.get<Array<TaskResponse>>('/api/v1/tasks', {
      active_only: params?.activeOnly,
      limit: params?.limit,
      offset: params?.offset,
    })
  },

  async getTask(taskId: string): Promise<TaskResponse> {
    return await apiClient.get<TaskResponse>(`/api/v1/tasks/${taskId}`)
  },

  async getTaskMetadata(taskId: string): Promise<TaskResponse> {
    return await apiClient.get<TaskResponse>(`/api/v1/tasks/${taskId}`)
  },

  async createTask(request: AgentTriggerRequest): Promise<AgentTriggerResponse> {
    return await apiClient.post<AgentTriggerResponse>('/api/v1/tasks', request)
  },

  async sendMessage(taskId: string, message: string): Promise<AgentTriggerResponse> {
    return await apiClient.post<AgentTriggerResponse>(`/api/v1/tasks/${taskId}/messages`, {
      message,
    })
  },

  // Deprecated: Use createTask instead
  triggerAgent(request: AgentTriggerRequest): Promise<AgentTriggerResponse> {
    return this.createTask(request)
  },

  // Deprecated: Use deleteTask instead
  async stopAgent(taskId: string): Promise<{ status: string; task_id: string }> {
    return await apiClient.post(`/api/v1/tasks/${taskId}/stop`)
  },

  async getTaskEvents(
    taskId: string,
    params?: {
      limit?: number
      offset?: number
    },
  ): Promise<Array<Event>> {
    return await apiClient.get<Array<Event>>(`/api/v1/tasks/${taskId}/events`, params)
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

  async startServer(taskId: string): Promise<ServerStatus> {
    return await apiClient.post<ServerStatus>(`/api/v1/tasks/${taskId}/server/start`)
  },

  async stopServer(taskId: string): Promise<ServerStatus> {
    return await apiClient.post<ServerStatus>(`/api/v1/tasks/${taskId}/server/stop`)
  },

  async getServerStatus(taskId: string): Promise<ServerStatus> {
    return await apiClient.get<ServerStatus>(`/api/v1/tasks/${taskId}/server/status`)
  },

  async getServerLogs(taskId: string): Promise<{ logs: string; lines: number }> {
    return await apiClient.get<{ logs: string; lines: number }>(
      `/api/v1/tasks/${taskId}/server/logs`,
    )
  },

  async mergeTask(taskId: string): Promise<{ status: string; message: string }> {
    return await apiClient.post<{ status: string; message: string }>(
      `/api/v1/tasks/${taskId}/merge`,
    )
  },
}
