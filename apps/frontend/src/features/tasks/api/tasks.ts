import type {
  CreateTaskRequest as ApiCreateTaskRequest,
  CreateVibeTaskRequest as ApiCreateVibeTaskRequest,
  SendMessageRequest as ApiSendMessageRequest,
} from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  sendMessage as sendMessageHandler,
  stopAgent as stopAgentHandler,
} from '@moatless/api-client/sandbox/agent/agent'
import { getAvailableCommands as getAvailableCommandsHandler } from '@moatless/api-client/sandbox/commands/commands'
import {
  deleteQueuedMessage as deleteQueuedMessageHandler,
  getMessages as getMessagesHandler,
} from '@moatless/api-client/sandbox/messages/messages'
import {
  closeTask as closeTaskHandler,
  createTask as createTaskHandler,
  createVibeTask as createVibeTaskHandler,
  deleteTask as deleteTaskHandler,
  getTask as getTaskHandler,
  listTasks as listTasksHandler,
  updateTaskMetadata as updateTaskMetadataHandler,
  updateVisibility as updateVisibilityHandler,
} from '@moatless/api-client/sandbox/tasks/tasks'
import type {
  CreateTaskRequest,
  CreateVibeTaskRequest,
  Message,
  MessageContext,
  SendMessageResponse,
  SimpleTaskResponse,
  Task,
  TaskCommandsResponse,
  TaskListResponse,
} from '@moatless/types'

export const tasksApi = {
  async getActiveTasks(): Promise<string[]> {
    const response = (await unwrapResponse(
      listTasksHandler({ status: 'active' }),
    )) as unknown as TaskListResponse
    return response.items.map((task) => task.taskId)
  },

  async listTasks(params?: {
    activeOnly?: boolean
    status?: 'open' | 'running' | 'error' | 'merged' | 'closed'
    limit?: number
    offset?: number
    tag?: string
    createdBy?: string
    repositoryId?: string
  }): Promise<TaskListResponse> {
    const result = await unwrapResponse(
      listTasksHandler({
        status: params?.status || (params?.activeOnly ? 'active' : undefined),
        limit: params?.limit,
        offset: params?.offset,
        tag: params?.tag,
        createdBy: params?.createdBy,
        repositoryId: params?.repositoryId,
      }),
    )
    return result as unknown as TaskListResponse
  },

  async getTask(taskId: string): Promise<Task> {
    const result = await unwrapResponse(getTaskHandler(taskId))
    return result as unknown as Task
  },

  async getTaskMetadata(taskId: string): Promise<Task> {
    const result = await unwrapResponse(getTaskHandler(taskId))
    return result as unknown as Task
  },

  async createTask(request: CreateTaskRequest): Promise<SimpleTaskResponse> {
    const result = await unwrapResponse(
      createTaskHandler(request as unknown as ApiCreateTaskRequest),
    )
    return result as unknown as SimpleTaskResponse
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
      contexts?: MessageContext[]
      toolUseId?: string
      toolName?: 'AskUserQuestion' | 'ExitPlanMode'
      toolResponse?: unknown
    },
  ): Promise<SendMessageResponse> {
    const result = await unwrapResponse(
      sendMessageHandler(taskId, variables as unknown as ApiSendMessageRequest),
    )
    return result as unknown as SendMessageResponse
  },

  // Deprecated: Use createTask instead
  triggerAgent(request: CreateTaskRequest): Promise<SimpleTaskResponse> {
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

  async getTaskMessages(taskId: string): Promise<Message[]> {
    const response = (await unwrapResponse(getMessagesHandler(taskId))) as unknown as {
      messages: Message[]
    }
    return response.messages
  },

  async getTaskCommands(taskId: string): Promise<TaskCommandsResponse> {
    const commands = await unwrapResponse(getAvailableCommandsHandler(taskId))
    return { commands } as unknown as TaskCommandsResponse
  },

  async closeTask(taskId: string): Promise<Task> {
    const result = await unwrapResponse(closeTaskHandler(taskId))
    return result as unknown as Task
  },

  async deleteTask(taskId: string): Promise<{ status: string }> {
    await unwrapResponse<void>(deleteTaskHandler(taskId))
    return { status: 'deleted' }
  },

  // TODO: Deploy endpoint not implemented in Rust backend
  deployTask(_taskId: string): Promise<{ success: boolean }> {
    return Promise.resolve({ success: false })
  },

  async createVibeTask(
    request: CreateVibeTaskRequest,
  ): Promise<{ taskId: string; status: string }> {
    const response = (await unwrapResponse(
      createVibeTaskHandler(request as unknown as ApiCreateVibeTaskRequest),
    )) as unknown as SimpleTaskResponse
    return { taskId: response.taskId, status: response.status }
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
