import type {
  SendMessageRequest as ApiSendMessageRequest,
  CloseTaskRequest,
  Connection,
  CreateTaskRequest,
  CreateTaskResponse,
  GetMessagesResponse,
  ListTasksResponse,
  MessageContextInput,
  SendMessageRequestToolResponse,
  SendMessageResponse,
  SubagentTranscriptResponse,
  TaskResponse,
  ToolCallDetailResponse,
  UiMessage,
  UpdateTaskTtlRequest,
} from '@moatless/api-client'
import { customInstance, unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  getMessages as getMessagesHandler,
  getSubagentTranscript as getSubagentTranscriptHandler,
  getToolCallDetail as getToolCallDetailHandler,
  sendMessage as sendMessageHandler,
} from '@moatless/api-client/sandbox/messages/messages'
import {
  closeTask as closeTaskHandler,
  createTask as createTaskHandler,
  deleteTask as deleteTaskHandler,
  getTaskConnections as getTaskConnectionsHandler,
  getTask as getTaskHandler,
  listTasks as listTasksHandler,
  reopenTask as reopenTaskHandler,
  stopTask as stopAgentHandler,
  updateTaskMetadata as updateTaskMetadataHandler,
  updateTtl as updateTaskTtlHandler,
  updateVisibility as updateVisibilityHandler,
} from '@moatless/api-client/sandbox/tasks/tasks'

export interface MessagesPage {
  messages: UiMessage[]
  nextBeforeId?: number | undefined
  hasMore: boolean
}

export interface TaskTokenTotals {
  inputTokens: number
  outputTokens: number
  cachedInputTokens: number
  cacheCreationInputTokens: number
  cacheReadInputTokens: number
  totalTokens: number
}

export interface TaskMessageTokenUsage {
  messageUuid: string
  timestamp: string
  turnNumber?: number | null
  model?: string | null
  inputTokens: number
  outputTokens: number
  cachedInputTokens: number
  totalTokens: number
}

export interface TaskToolCallStats {
  toolName: string
  totalCalls: number
  failedCalls: number
}

export interface TaskSkillUsage {
  skillName: string
  totalCalls: number
}

export interface TaskStatisticsResponse {
  taskId: string
  agentType: string
  model?: string | null
  tokenTotals: TaskTokenTotals
  messageTokenUsage: TaskMessageTokenUsage[]
  toolCalls: TaskToolCallStats[]
  skills: TaskSkillUsage[]
}

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
    followedBy?: string
    repositoryId?: string
    parentTaskId?: string
  }): Promise<ListTasksResponse> {
    const status = params?.status || (params?.activeOnly ? 'open' : undefined)
    return await unwrapResponse(
      listTasksHandler({
        ...(status !== undefined && { status }),
        ...(params?.limit !== undefined && { limit: params.limit }),
        ...(params?.offset !== undefined && { offset: params.offset }),
        ...(params?.tag !== undefined && { tag: params.tag }),
        ...(params?.createdBy !== undefined && { createdBy: params.createdBy }),
        ...(params?.followedBy !== undefined && { followedBy: params.followedBy }),
        ...(params?.repositoryId !== undefined && { repositoryId: params.repositoryId }),
        ...(params?.parentTaskId !== undefined && { parentTaskId: params.parentTaskId }),
      }),
    )
  },

  async getTask(taskId: string): Promise<TaskResponse> {
    return await unwrapResponse(getTaskHandler(taskId))
  },

  async getTaskMetadata(taskId: string): Promise<TaskResponse> {
    return await unwrapResponse(getTaskHandler(taskId))
  },

  async getTaskConnections(taskId: string, opts?: { refresh?: boolean }): Promise<Connection[]> {
    const refresh = opts?.refresh
    return await unwrapResponse(
      getTaskConnectionsHandler(taskId, {
        ...(refresh !== undefined && { refresh }),
      }),
    )
  },

  async getTaskStatistics(taskId: string): Promise<TaskStatisticsResponse> {
    return await customInstance<TaskStatisticsResponse>(`/api/v1/tasks/${taskId}/statistics`)
  },

  async createTask(request: CreateTaskRequest): Promise<CreateTaskResponse> {
    return await unwrapResponse(createTaskHandler(request))
  },

  async sendMessage(
    taskId: string,
    variables: {
      message: string
      resume?: boolean | undefined
      model?: string | undefined
      fileIds?: string[] | undefined
      /**
       * Optimistic-render only — never sent to the backend. Stripped here
       * so the field can't leak into the request body.
       */
      fileUploads?:
        | Array<{ fileId: string; filename: string; mimeType: string; size: number }>
        | undefined
      agentMode?: string | undefined
      contexts?: MessageContextInput[] | undefined
      toolUseId?: string | undefined
      toolName?: 'AskUserQuestion' | 'ExitPlanMode' | undefined
      toolResponse?: unknown
      startFresh?: boolean | undefined
    },
  ): Promise<SendMessageResponse> {
    // fileUploads is a UI-only optimistic-render field and is dropped here.
    // Build the request explicitly so it is typed as ApiSendMessageRequest by
    // construction: omitting `undefined` fields (rather than casting) reconciles
    // the local `| undefined` optionals with the generated `| null` nullables.
    const payload: ApiSendMessageRequest = {
      message: variables.message,
      ...(variables.resume !== undefined && { resume: variables.resume }),
      ...(variables.model !== undefined && { model: variables.model }),
      ...(variables.fileIds !== undefined && { fileIds: variables.fileIds }),
      ...(variables.agentMode !== undefined && { agentMode: variables.agentMode }),
      ...(variables.contexts !== undefined && { contexts: variables.contexts }),
      ...(variables.toolUseId !== undefined && { toolUseId: variables.toolUseId }),
      ...(variables.toolName !== undefined && { toolName: variables.toolName }),
      ...(variables.toolResponse !== undefined && {
        // The tool-reply payload is an opaque record (answers/approval). The
        // generated request type is an index-signature record that concrete
        // response interfaces satisfy structurally but not nominally, so map
        // this single field explicitly rather than casting the whole request.
        toolResponse: variables.toolResponse as SendMessageRequestToolResponse,
      }),
      ...(variables.startFresh !== undefined && { startFresh: variables.startFresh }),
    }
    return await unwrapResponse(sendMessageHandler(taskId, payload))
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

  async getTaskMessages(
    taskId: string,
    params?: { beforeId?: number; limit?: number },
  ): Promise<MessagesPage> {
    const response = await unwrapResponse<GetMessagesResponse>(
      getMessagesHandler(taskId, {
        ...(params?.beforeId !== undefined && { before_id: params.beforeId }),
        ...(params?.limit !== undefined && { limit: params.limit }),
        // Slim wire payload: full tool_input + result are fetched on demand
        // via `getToolCallDetail` when a tool call is expanded.
        summary: true,
      }),
    )
    return {
      messages: response.messages,
      nextBeforeId: response.nextBeforeId ?? undefined,
      hasMore: response.hasMore,
    }
  },

  async getToolCallDetail(taskId: string, toolUseId: string): Promise<ToolCallDetailResponse> {
    return await unwrapResponse(getToolCallDetailHandler(taskId, toolUseId))
  },

  async getSubagentTranscript(
    taskId: string,
    toolUseId: string,
  ): Promise<SubagentTranscriptResponse> {
    return await unwrapResponse(getSubagentTranscriptHandler(taskId, toolUseId))
  },

  async closeTask(taskId: string, closeChildren?: boolean): Promise<TaskResponse> {
    const request: CloseTaskRequest = closeChildren === undefined ? {} : { closeChildren }
    return await unwrapResponse(closeTaskHandler(taskId, request))
  },

  async reopenTask(taskId: string): Promise<TaskResponse> {
    return await unwrapResponse(reopenTaskHandler(taskId))
  },

  // Mark the task's conversation as read for the current user (clears the
  // unread indicator). Called via the shared instance rather than a generated
  // handler so it stays in sync with the backend regardless of client regen.
  async markTaskRead(taskId: string): Promise<void> {
    await customInstance<void>(`/api/v1/tasks/${taskId}/read`, { method: 'POST' })
  },

  // Follow a task so it shows up in the user's personal task list. Called via
  // the shared instance rather than a generated handler so it stays in sync
  // with the backend regardless of client regen.
  async followTask(taskId: string): Promise<{ following: boolean }> {
    return await customInstance<{ following: boolean }>(`/api/v1/tasks/${taskId}/follow`, {
      method: 'POST',
    })
  },

  // Stop following a task, removing it from the user's personal task list.
  async unfollowTask(taskId: string): Promise<{ following: boolean }> {
    return await customInstance<{ following: boolean }>(`/api/v1/tasks/${taskId}/follow`, {
      method: 'DELETE',
    })
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

  async updateTaskTtl(taskId: string, request: UpdateTaskTtlRequest): Promise<{ success: true }> {
    await unwrapResponse(updateTaskTtlHandler(taskId, request))
    return { success: true }
  },
}
