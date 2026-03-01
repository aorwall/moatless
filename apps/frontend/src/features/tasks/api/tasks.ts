import type {
  SendMessageRequest as ApiSendMessageRequest,
  CommandResponse,
  CreateTaskRequest,
  CreateTaskResponse,
  ListTasksResponse,
  MessageContextInput,
  SendMessageResponse,
  TaskResponse,
  UiMessage,
} from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { getAvailableCommands as getAvailableCommandsHandler } from '@moatless/api-client/sandbox/commands/commands'
import {
  deleteQueuedMessage as deleteQueuedMessageHandler,
  getMessages as getMessagesHandler,
  sendMessage as sendMessageHandler,
} from '@moatless/api-client/sandbox/messages/messages'
import {
  closeTask as closeTaskHandler,
  createTask as createTaskHandler,
  deleteTask as deleteTaskHandler,
  getTask as getTaskHandler,
  listTasks as listTasksHandler,
  stopTask as stopAgentHandler,
  updateTaskMetadata as updateTaskMetadataHandler,
  updateVisibility as updateVisibilityHandler,
} from '@moatless/api-client/sandbox/tasks/tasks'

export const tasksApi = {
  async getActiveTasks(): Promise<string[]> {
    const response = await unwrapResponse<ListTasksResponse>(listTasksHandler({ status: 'open' }))
    return response.items.map((task) => task.taskId)
  },

  async listTasks(params?: {
    activeOnly?: boolean
    status?: 'open' | 'error' | 'closed'
    limit?: number
    offset?: number
    tag?: string
    createdBy?: string
    repositoryId?: string
  }): Promise<ListTasksResponse> {
    return await unwrapResponse(
      listTasksHandler({
        status: params?.status || (params?.activeOnly ? 'open' : undefined),
        limit: params?.limit,
        offset: params?.offset,
        tag: params?.tag,
        createdBy: params?.createdBy,
        repositoryId: params?.repositoryId,
      }),
    )
  },

  async getTask(taskId: string): Promise<TaskResponse> {
    return await unwrapResponse(getTaskHandler(taskId))
  },

  async getTaskMetadata(taskId: string): Promise<TaskResponse> {
    return await unwrapResponse(getTaskHandler(taskId))
  },

  async createTask(request: CreateTaskRequest): Promise<CreateTaskResponse> {
    return await unwrapResponse(createTaskHandler(request))
  },

  async sendMessage(
    taskId: string,
    variables: {
      message: string
      resume?: boolean
      model?: string
      fileIds?: string[]
      commandName?: string
      agentMode?: string
      contexts?: MessageContextInput[]
      toolUseId?: string
      toolName?: 'AskUserQuestion' | 'ExitPlanMode'
      toolResponse?: unknown
    },
  ): Promise<SendMessageResponse> {
    return await unwrapResponse(
      sendMessageHandler(taskId, variables as unknown as ApiSendMessageRequest),
    )
  },

  // Deprecated: Use createTask instead
  triggerAgent(request: CreateTaskRequest): Promise<CreateTaskResponse> {
    return this.createTask(request)
  },

  // Deprecated: Use deleteTask instead
  async stopAgent(taskId: string): Promise<{ status: string; taskId: string }> {
    await unwrapResponse<void>(stopAgentHandler(taskId))
    return { status: 'stopped', taskId }
  },

  async deleteMessage(taskId: string, messageId: string): Promise<{ status: 'deleted' }> {
    await unwrapResponse<void>(deleteQueuedMessageHandler(taskId, messageId))
    return { status: 'deleted' }
  },

  async getTaskMessages(taskId: string): Promise<UiMessage[]> {
    const response = await unwrapResponse<{ messages: UiMessage[] }>(getMessagesHandler(taskId))
    return response.messages
  },

  async getTaskCommands(taskId: string): Promise<CommandResponse[]> {
    return await unwrapResponse(getAvailableCommandsHandler(taskId))
  },

  async closeTask(taskId: string): Promise<TaskResponse> {
    return await unwrapResponse(closeTaskHandler(taskId))
  },

  async deleteTask(taskId: string): Promise<{ status: string }> {
    await unwrapResponse<void>(deleteTaskHandler(taskId))
    return { status: 'deleted' }
  },

  // TODO: Deploy endpoint not implemented in Rust backend
  deployTask(_taskId: string): Promise<{ success: boolean; siteUrl?: string; error?: string }> {
    return Promise.resolve({ success: false })
  },

  async updateTaskVisibility(
    taskId: string,
    visibility: 'private' | 'public',
  ): Promise<{ success: true }> {
    await unwrapResponse(updateVisibilityHandler(taskId, { visibility }))
    return { success: true }
  },

  async updateTaskMetadata(
    taskId: string,
    updates: { name?: string; description?: string },
  ): Promise<{ success: boolean }> {
    await unwrapResponse(updateTaskMetadataHandler(taskId, updates))
    return { success: true }
  },
}
