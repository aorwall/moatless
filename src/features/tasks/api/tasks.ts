import { apiClient } from '@/lib/api/client';
import type { AgentTriggerRequest, AgentTriggerResponse } from '@/types/agent';
import type {
  FileSearchRequest,
  FileSearchResponse,
} from '@/types/environment';
import type { Event } from '@/types/event.ts';
import type { GitDiffResponse } from '@/types/git-diff';
import type { TaskMessages, TaskMetadata } from '@/types/task';

export const tasksApi = {
  async getActiveTasks(): Promise<Array<string>> {
    const tasks = await apiClient.get<Array<TaskMetadata>>('/api/v1/tasks', {
      active_only: true,
    });
    return tasks.map((task) => task.task_id);
  },

  async listTasks(params?: {
    activeOnly?: boolean;
    limit?: number;
    offset?: number;
  }): Promise<Array<TaskMetadata>> {
    return await apiClient.get<Array<TaskMetadata>>('/api/v1/tasks', {
      active_only: params?.activeOnly,
      limit: params?.limit,
      offset: params?.offset,
    });
  },

  async getTask(taskId: string): Promise<TaskMetadata> {
    return await apiClient.get<TaskMetadata>(`/api/v1/tasks/${taskId}`);
  },

  async getTaskMessages(
    taskId: string,
    params?: {
      limit?: number;
      offset?: number;
    },
  ): Promise<TaskMessages> {
    return await apiClient.get<TaskMessages>(
      `/api/v1/tasks/${taskId}/messages`,
      params,
    );
  },

  async getTaskMetadata(taskId: string): Promise<TaskMetadata> {
    return await apiClient.get<TaskMetadata>(`/api/v1/tasks/${taskId}`);
  },

  async createTask(
    request: AgentTriggerRequest,
  ): Promise<AgentTriggerResponse> {
    return await apiClient.post<AgentTriggerResponse>('/api/v1/tasks', request);
  },

  async sendMessage(
    taskId: string,
    message: string,
  ): Promise<AgentTriggerResponse> {
    return await apiClient.post<AgentTriggerResponse>(
      `/api/v1/tasks/${taskId}/messages`,
      { message },
    );
  },

  // Deprecated: Use createTask instead
  triggerAgent(request: AgentTriggerRequest): Promise<AgentTriggerResponse> {
    return this.createTask(request);
  },

  // Deprecated: Use deleteTask instead
  async stopAgent(
    taskId: string,
  ): Promise<{ status: string; task_id: string }> {
    return await apiClient.delete(`/api/v1/tasks/${taskId}`);
  },

  async getTaskEvents(
    taskId: string,
    params?: {
      limit?: number;
      offset?: number;
    },
  ): Promise<Array<Event>> {
    return await apiClient.get<Array<Event>>(
      `/api/v1/tasks/${taskId}/events`,
      params,
    );
  },

  async getTaskGitDiff(taskId: string): Promise<GitDiffResponse> {
    return await apiClient.get<GitDiffResponse>(
      `/api/v1/tasks/${taskId}/git-diff`,
    );
  },

  async searchTaskFiles(
    taskId: string,
    request: FileSearchRequest,
  ): Promise<FileSearchResponse> {
    return await apiClient.post<FileSearchResponse>(
      `/api/v1/tasks/${taskId}/search-files`,
      request,
    );
  },
};
