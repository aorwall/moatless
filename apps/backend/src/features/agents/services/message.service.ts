import { randomUUID } from 'node:crypto'
import type {
  AgentEvent,
  AgentMessage,
  AssistantMessage,
  AssistantMessageContent,
  ClaudeCodeSDKMessage,
  Event,
  Message,
  MessageContext,
  Task,
  TaskIteration,
  ToolResult,
  UserMessage,
} from '@moatless/types'
import { isTextContent, isToolResultContent, isToolUseContent } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { ClaudeService } from '../../claude/services/claude.service'
import { FileStorageService } from '../../file-storage'
import { TaskRepository } from '../../tasks/db/task.repository'
import { TaskMessageRepository } from '../../tasks/db/task-message.repository'

// Type guard for agent message events
function isAgentMessageEvent(event: Event): event is AgentEvent & { subType: 'message' } {
  return event.type === 'agent' && event.subType === 'message'
}

// Type for message content blocks
interface MessageContentBlock {
  type: string
  text?: string
}

@injectable()
export class MessageService {
  constructor(
    @inject(ClaudeService) private claudeService: ClaudeService,
    @inject(TaskMessageRepository) private messageRepository: TaskMessageRepository,
    @inject(TaskRepository) private taskRepository: TaskRepository,
    @inject(FileStorageService) private uploadService: FileStorageService,
  ) {}

  /**
   * Normalize message content to always be an array format
   * Converts string content to [{type: 'text', text: content}]
   */
  private normalizeMessageContent(
    content: string | { type: string; text?: string; [key: string]: unknown }[],
  ): { type: string; text?: string; [key: string]: unknown }[] {
    if (typeof content === 'string') {
      return [{ type: 'text', text: content }]
    }
    return content
  }

  /**
   * Get task messages by task ID
   * This is the main public method that should be used by routes
   * Returns transformed Messages for UI consumption
   */
  async getTaskMessages(taskId: string): Promise<Message[]> {
    // Prefer persisted transcripts when available
    const agentMessages = await this.getMessagesFromRepository(taskId)
    if (agentMessages.length > 0) {
      return this.transformAgentMessagesToMessages(agentMessages)
    }

    // Fall back to task metadata + Claude session replay when repository is empty
    logger.debug(
      {
        taskId,
      },
      'No repository transcript found, falling back to task sources',
    )
    const task = await this.taskRepository.get(taskId)
    if (!task) {
      throw new Error(`Task ${taskId} not found`)
    }

    if (task.agentType !== 'claude-code') {
      throw new Error('Only Claude Code tasks are supported for message reading')
    }

    return await this.readMessagesFromClaudeDirectory(task)
  }

  /**
   * Get messages from the relational repository backing transcripts
   */
  async getMessagesFromRepository(taskId: string): Promise<AgentMessage[]> {
    try {
      const messages = await this.messageRepository.getMessages(taskId)
      logger.debug(
        {
          taskId,
          messageCount: messages.length,
        },
        'Read messages from repository',
      )
      return messages
    } catch (error) {
      logger.warn(
        {
          taskId,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to read messages from repository',
      )
      return []
    }
  }

  /**
   * Save a user message to the message repository
   */
  async saveUserMessage(
    taskId: string,
    message: string,
    sessionId?: string,
    iterationNumber?: number,
    fileIds?: string[],
    contexts?: MessageContext[],
  ): Promise<void> {
    try {
      const userMessage: AgentMessage = {
        uuid: randomUUID(),
        timestamp: new Date().toISOString(),
        userType: 'user',
        type: 'user',
        message: {
          role: 'user',
          content: [{ type: 'text', text: message }],
        },
        sessionId,
        iterationNumber,
      }

      // Add file attachments if provided
      if (fileIds && fileIds.length > 0) {
        userMessage.attachments = []
        for (const fileId of fileIds) {
          // Get file metadata from upload service
          try {
            const metadata = await this.uploadService.getFileMetadata(fileId)
            userMessage.attachments.push({
              fileId: metadata.fileId,
              filename: metadata.filename,
              mimeType: metadata.mimeType,
            })
          } catch (error) {
            logger.warn(
              {
                fileId,
                error: error instanceof Error ? error.message : String(error),
              },
              'Failed to get file metadata for attachment',
            )
            // Skip this attachment if metadata can't be retrieved
          }
        }
      }

      // Add contexts if provided
      if (contexts && contexts.length > 0) {
        userMessage.contexts = contexts
      }

      await this.messageRepository.append(taskId, userMessage)
      logger.debug(
        {
          taskId,
          messageId: userMessage.uuid,
          iterationNumber,
          hasSessionId: Boolean(sessionId),
          hasAttachments: (userMessage.attachments?.length ?? 0) > 0,
          hasContexts: (userMessage.contexts?.length ?? 0) > 0,
        },
        'User message persisted',
      )
    } catch (error) {
      logger.warn(
        {
          taskId,
          iterationNumber,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to persist user message',
      )
      throw error
    }
  }

  /**
   * Save an agent message from an event to the repository
   */
  async saveAgentMessage(taskId: string, event: Event): Promise<void> {
    try {
      if (!isAgentMessageEvent(event)) {
        return
      }

      const sdkMessage = event.payload as ClaudeCodeSDKMessage

      // Check if this is a duplicate initial message (only for user messages)
      if (sdkMessage.type === 'user') {
        const isDuplicate = await this.isInitialMessageDuplicate(taskId, event, sdkMessage)
        if (isDuplicate) {
          return
        }
      }

      const agentMessage = this.convertEventToAgentMessage(event)
      if (agentMessage) {
        await this.messageRepository.append(taskId, agentMessage)
        logger.debug(
          {
            taskId,
            messageType: agentMessage.type,
            timestamp: agentMessage.timestamp,
          },
          'Agent message persisted',
        )
      }
    } catch (error) {
      logger.warn(
        {
          taskId,
          eventType: event.type,
          eventSubType: 'subType' in event ? event.subType : undefined,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to persist agent message',
      )
      throw error
    }
  }

  /**
   * Check if a user message is a duplicate of the iteration's initial message
   */
  private async isInitialMessageDuplicate(
    taskId: string,
    event: Event,
    sdkMessage: ClaudeCodeSDKMessage,
  ): Promise<boolean> {
    const iterationId =
      event.type === 'agent' && 'iterationId' in event ? event.iterationId : undefined

    // Only user messages have the structure we need to check
    if (!(iterationId && sdkMessage.type === 'user' && 'message' in sdkMessage)) {
      return false
    }

    const task = await this.taskRepository.get(taskId)
    if (!task) {
      return false
    }

    const iteration = task.iterations.find(
      (it: TaskIteration) => it.iterationNumber === iterationId,
    )
    if (!iteration?.initialMessage) {
      return false
    }

    const content = sdkMessage.message.content
    const messageContent =
      typeof content === 'string'
        ? content
        : Array.isArray(content)
          ? content
              .filter(
                (c): c is MessageContentBlock =>
                  typeof c === 'object' && c !== null && 'type' in c && c.type === 'text',
              )
              .map((c) => c.text || '')
              .join('\n')
          : ''

    if (messageContent === iteration.initialMessage) {
      logger.debug(
        {
          taskId,
          iterationNumber: iteration.iterationNumber,
        },
        'Skipping duplicate initial message',
      )
      return true
    }

    return false
  }

  /**
   * Read messages directly from Claude directory
   * @internal - Made public for testing, but should not be used directly in production code
   */
  async readMessagesFromClaudeDirectory(task: Task): Promise<Message[]> {
    try {
      const allAgentMessages = await this.collectAllAgentMessages(task)

      if (allAgentMessages.length > 0) {
        logger.debug(
          {
            taskId: task.taskId,
            messageCount: allAgentMessages.length,
          },
          'Found Claude messages',
        )
        return this.transformAgentMessagesToMessages(allAgentMessages)
      }

      logger.info(
        {
          taskId: task.taskId,
        },
        'No Claude messages found, falling back to task iterations',
      )
      return this.createMessagesFromTaskIterations(task)
    } catch (error) {
      logger.error({ err: error as Error, taskId: task.taskId }, 'Error reading Claude messages')
      return this.createMessagesFromTaskIterations(task)
    }
  }

  private async collectAllAgentMessages(task: Task): Promise<AgentMessage[]> {
    const allAgentMessages: AgentMessage[] = []

    for (const iteration of task.iterations) {
      if (!iteration.agentSessionId) {
        logger.debug(
          {
            taskId: task.taskId,
            iterationNumber: iteration.iterationNumber,
          },
          'No agent session ID for iteration',
        )
        continue
      }

      const agentMessages = await this.claudeService.readClaudeMessages(iteration.agentSessionId)
      const messagesToAdd = this.processIterationMessages(task, iteration, agentMessages)
      allAgentMessages.push(...messagesToAdd)
    }

    return allAgentMessages
  }

  private processIterationMessages(
    task: Task,
    iteration: TaskIteration,
    agentMessages: AgentMessage[],
  ): AgentMessage[] {
    if (agentMessages.length === 0) {
      return this.createFallbackMessagesForIteration(iteration)
    }

    return this.mergeIterationWithAgentMessages(task, iteration, agentMessages)
  }

  private createFallbackMessagesForIteration(iteration: TaskIteration): AgentMessage[] {
    const messages: AgentMessage[] = []

    if (iteration.initialMessage) {
      messages.push({
        userType: 'user',
        uuid: crypto.randomUUID(),
        timestamp: iteration.startTime.toISOString(),
        type: 'user',
        message: {
          role: 'user',
          content: iteration.initialMessage,
        },
      })
    }

    if (iteration.result) {
      const resultContent = this.parseMessageFromIteration(iteration)
      if (resultContent) {
        messages.push({
          userType: 'user',
          uuid: crypto.randomUUID(),
          timestamp: (iteration.endTime || iteration.startTime).toISOString(),
          type: 'assistant',
          message: {
            type: 'message',
            role: 'assistant',
            content: [{ type: 'text', text: resultContent }],
          },
        })
      }
    }

    return messages
  }

  private mergeIterationWithAgentMessages(
    task: Task,
    iteration: TaskIteration,
    agentMessages: AgentMessage[],
  ): AgentMessage[] {
    if (!iteration.initialMessage) {
      return agentMessages
    }

    logger.debug(
      {
        taskId: task.taskId,
        iterationNumber: iteration.iterationNumber,
        agentSessionId: iteration.agentSessionId,
      },
      'Adding iteration initial message',
    )

    const messagesToAdd: AgentMessage[] = []
    const firstUserMessageIndex = this.findFirstUserMessageIndex(task, iteration, agentMessages)
    const replacementUuid = (() => {
      if (firstUserMessageIndex >= 0 && agentMessages[firstUserMessageIndex]) {
        return agentMessages[firstUserMessageIndex]?.uuid
      }
      if (agentMessages.length > 0) {
        return agentMessages[0]?.uuid ?? crypto.randomUUID()
      }
      return crypto.randomUUID()
    })()

    const initialUserMessage: AgentMessage = {
      userType: 'user',
      uuid: replacementUuid,
      timestamp: iteration.startTime.toISOString(),
      type: 'user',
      message: {
        role: 'user',
        content: iteration.initialMessage,
      },
    }
    messagesToAdd.push(initialUserMessage)

    this.addNonDuplicateMessages(messagesToAdd, agentMessages, firstUserMessageIndex)

    return messagesToAdd
  }

  private findFirstUserMessageIndex(
    task: Task,
    iteration: TaskIteration,
    agentMessages: AgentMessage[],
  ): number {
    for (let i = 0; i < agentMessages.length; i++) {
      const agentMsg = agentMessages[i]
      if (agentMsg?.type === 'user' && this.hasTextContent(agentMsg)) {
        logger.debug(
          {
            index: i,
            taskId: task.taskId,
            iterationNumber: iteration.iterationNumber,
          },
          'Found first user message to skip at index',
        )
        return i
      }
    }
    return -1
  }

  private hasTextContent(agentMsg: AgentMessage): boolean {
    if (!agentMsg.message?.content) {
      return false
    }

    const content = agentMsg.message.content

    if (typeof content === 'string') {
      return true
    }

    if (Array.isArray(content)) {
      return content.some(
        (c) => typeof c === 'object' && c !== null && 'type' in c && c.type === 'text',
      )
    }

    return false
  }

  private addNonDuplicateMessages(
    messagesToAdd: AgentMessage[],
    agentMessages: AgentMessage[],
    skipIndex: number,
  ): void {
    for (let i = 0; i < agentMessages.length; i++) {
      const message = agentMessages[i]
      if (i !== skipIndex && message) {
        messagesToAdd.push(message)
      }
    }
  }

  /**
   * Create messages from a single task iteration
   */
  private createMessagesFromSingleIteration(iteration: TaskIteration): Message[] {
    const messages: Message[] = []

    // Skip iterations without messages
    if (!(iteration.initialMessage || iteration.result)) {
      return messages
    }

    // Add user message if exists
    if (iteration.initialMessage) {
      const userMessage: UserMessage = {
        id: `iteration-${iteration.iterationNumber}-user`,
        role: 'user',
        content: iteration.initialMessage,
        timestamp: iteration.startTime,
        eventId: `iteration-${iteration.iterationNumber}-user`,
      }
      messages.push(userMessage)
    }

    // Add assistant message - always try to parse message from iteration
    const assistantContent = this.parseMessageFromIteration(iteration)
    if (assistantContent) {
      const assistantMessage: AssistantMessage = {
        id: `iteration-${iteration.iterationNumber}-assistant`,
        role: 'assistant',
        content: assistantContent,
        timestamp: iteration.endTime || iteration.startTime,
        eventId: `iteration-${iteration.iterationNumber}-assistant`,
      }
      messages.push(assistantMessage)
    }

    return messages
  }

  /**
   * Transform agent messages directly into UI messages (bypassing events)
   */
  private transformAgentMessagesToMessages(agentMessages: AgentMessage[]): Message[] {
    const seenUuids = new Set<string>()
    const toolResultsMap = this.buildToolResultsMap(agentMessages, seenUuids)
    return this.transformMessagesWithToolResults(agentMessages, toolResultsMap, seenUuids)
  }

  private buildToolResultsMap(
    agentMessages: AgentMessage[],
    seenUuids: Set<string>,
  ): Map<string, AgentMessage> {
    const toolResultsMap = new Map<string, AgentMessage>()

    for (const agentMessage of agentMessages) {
      if (seenUuids.has(agentMessage.uuid)) {
        continue
      }
      seenUuids.add(agentMessage.uuid)

      if (agentMessage.type === 'user' && agentMessage.message) {
        this.extractToolResults(agentMessage, toolResultsMap)
      }
    }

    return toolResultsMap
  }

  private extractToolResults(
    agentMessage: AgentMessage,
    toolResultsMap: Map<string, AgentMessage>,
  ): void {
    const apiMessage = agentMessage.message
    if (apiMessage?.content && Array.isArray(apiMessage.content)) {
      for (const content of apiMessage.content) {
        if (isToolResultContent(content)) {
          toolResultsMap.set(content.tool_use_id, agentMessage)
        }
      }
    }
  }

  private transformMessagesWithToolResults(
    agentMessages: AgentMessage[],
    toolResultsMap: Map<string, AgentMessage>,
    seenUuids: Set<string>,
  ): Message[] {
    const messages: Message[] = []
    const processedIds = new Set<string>()

    for (const agentMessage of agentMessages) {
      if (processedIds.has(agentMessage.uuid) || !seenUuids.has(agentMessage.uuid)) {
        continue
      }

      const message = this.transformSingleAgentMessage(agentMessage, toolResultsMap, processedIds)
      if (message) {
        messages.push(message)
      }
    }

    return messages
  }

  private transformSingleAgentMessage(
    agentMessage: AgentMessage,
    toolResultsMap: Map<string, AgentMessage>,
    processedIds: Set<string>,
  ): Message | null {
    if (agentMessage.type === 'assistant' && agentMessage.message) {
      return this.transformAgentAssistantMessage(agentMessage, toolResultsMap, processedIds)
    }

    if (agentMessage.type === 'user' && agentMessage.message) {
      return this.transformUserMessageIfNotToolResult(agentMessage)
    }

    return null
  }

  private transformUserMessageIfNotToolResult(agentMessage: AgentMessage): Message | null {
    const apiMessage = agentMessage.message
    if (apiMessage?.content && Array.isArray(apiMessage.content)) {
      const hasToolResult = apiMessage.content.some(isToolResultContent)
      if (hasToolResult) {
        return null
      }
    }

    return this.transformAgentUserMessage(agentMessage)
  }

  /**
   * Create messages from task iterations when no Claude files exist
   */
  private createMessagesFromTaskIterations(task: Task): Message[] {
    const messages: Message[] = []

    for (const iteration of task.iterations) {
      const iterationMessages = this.createMessagesFromSingleIteration(iteration)
      messages.push(...iterationMessages)
    }

    logger.debug(
      {
        taskId: task.taskId,
        totalIterations: task.iterations.length,
        messagesCreated: messages.length,
      },
      'Created messages from task iterations',
    )

    return messages
  }

  /**
   * Parse message content from a task iteration
   * Tries to extract the actual message from the iteration result
   */
  private parseMessageFromIteration(iteration: TaskIteration): string | null {
    if (!iteration.result) {
      return null
    }

    // Try to parse the result as a message object first
    try {
      const parsed = JSON.parse(iteration.result)

      // Check if it's a message object with content
      if (parsed && typeof parsed === 'object') {
        const obj = parsed as Record<string, unknown>
        // Look for common message content fields
        if (typeof obj['content'] === 'string') {
          return obj['content']
        }
        if (typeof obj['message'] === 'string') {
          return obj['message']
        }
        if (typeof obj['text'] === 'string') {
          return obj['text']
        }
        // If it's an array, try to extract text content
        if (Array.isArray(obj['content'])) {
          const textContent = obj['content']
            .filter(
              (item): item is MessageContentBlock =>
                item !== null && typeof item === 'object' && 'type' in item && item.type === 'text',
            )
            .map((item) => item.text || '')
            .filter((text) => text.length > 0)
            .join('\n')
          if (textContent.trim()) {
            return textContent
          }
        }
      }
    } catch {
      // If parsing fails, treat as plain text
    }

    // If not parseable as JSON or no structured content found, return the raw result
    return iteration.result
  }

  private transformAgentAssistantMessage(
    agentMessage: AgentMessage,
    toolResultsMap: Map<string, AgentMessage>,
    processedIds: Set<string>,
  ): AssistantMessage | null {
    if (agentMessage.type !== 'assistant' || !agentMessage.message) {
      return null
    }

    const apiMessage = agentMessage.message
    const content = this.extractAssistantMessageContent(apiMessage, toolResultsMap, processedIds)

    return {
      id: agentMessage.uuid,
      role: 'assistant',
      content: content.length > 0 ? content : [],
      timestamp: new Date(agentMessage.timestamp),
      eventId: agentMessage.uuid,
      ...(agentMessage.isVisibleInTranscriptOnly !== undefined && {
        isVisibleInTranscriptOnly: agentMessage.isVisibleInTranscriptOnly,
      }),
      ...(agentMessage.isCompactSummary !== undefined && {
        isCompactSummary: agentMessage.isCompactSummary,
      }),
    }
  }

  private extractAssistantMessageContent(
    apiMessage: AgentMessage['message'],
    toolResultsMap: Map<string, AgentMessage>,
    processedIds: Set<string>,
  ): AssistantMessageContent[] {
    if (typeof apiMessage.content === 'string') {
      return this.handleStringContent(apiMessage)
    }

    if (apiMessage.content && Array.isArray(apiMessage.content)) {
      return this.handleArrayContent(apiMessage.content, toolResultsMap, processedIds)
    }

    return []
  }

  private handleStringContent(apiMessage: AgentMessage['message']): AssistantMessageContent[] {
    if (!apiMessage.content || typeof apiMessage.content !== 'string') {
      return []
    }

    logger.warn(
      {
        messageId: 'id' in apiMessage ? apiMessage.id : undefined,
      },
      'Found string content in assistant message, should be array (old data)',
    )

    return [
      {
        type: 'text',
        text: apiMessage.content,
      },
    ]
  }

  private handleArrayContent(
    contentArray: unknown[],
    toolResultsMap: Map<string, AgentMessage>,
    processedIds: Set<string>,
  ): AssistantMessageContent[] {
    const content: AssistantMessageContent[] = []

    for (const messageContent of contentArray) {
      if (isTextContent(messageContent)) {
        content.push({
          type: 'text',
          text: messageContent.text,
        })
      } else if (isToolUseContent(messageContent)) {
        const toolUseContent = this.createToolUseContent(
          messageContent,
          toolResultsMap,
          processedIds,
        )
        content.push(toolUseContent)
      }
    }

    return content
  }

  private createToolUseContent(
    messageContent: { id: string; name: string; input: Record<string, unknown> },
    toolResultsMap: Map<string, AgentMessage>,
    processedIds: Set<string>,
  ): AssistantMessageContent {
    const toolUseId = messageContent.id
    const toolResult = toolResultsMap.get(toolUseId)
    const toolResultContent = this.extractToolResultContent(toolResult, processedIds)

    return {
      type: 'tool_use',
      tool_name: messageContent.name,
      tool_input: messageContent.input,
      tool_use_id: toolUseId,
      tool_result: toolResultContent,
    }
  }

  private extractToolResultContent(
    toolResult: AgentMessage | undefined,
    processedIds: Set<string>,
  ): ToolResult | undefined {
    if (!toolResult || toolResult.type !== 'user') {
      return undefined
    }

    processedIds.add(toolResult.uuid)

    const toolResultAPI = toolResult.message
    if (!(toolResultAPI.content && Array.isArray(toolResultAPI.content))) {
      return undefined
    }

    const resultContent = toolResultAPI.content.find(isToolResultContent)
    if (!resultContent) {
      return undefined
    }

    return {
      content: this.normalizeToolResultContent(resultContent.content),
      is_error: resultContent.is_error,
      eventId: toolResult.uuid,
    }
  }

  private normalizeToolResultContent(content: unknown): string {
    if (Array.isArray(content)) {
      return content.map((c) => (isTextContent(c) ? c.text : JSON.stringify(c))).join('\n')
    }

    if (typeof content === 'string') {
      return content
    }

    return JSON.stringify(content)
  }

  private transformAgentUserMessage(agentMessage: AgentMessage): UserMessage | null {
    if (agentMessage.type !== 'user' || !agentMessage.message) {
      return null
    }

    const apiMessage = agentMessage.message

    // Extract text content from user message
    let textContent = ''
    if (apiMessage.content && Array.isArray(apiMessage.content)) {
      textContent = apiMessage.content
        .filter(isTextContent)
        .map((c) => c.text)
        .join('\n')
    } else {
      logger.warn(
        {
          messageId: agentMessage.uuid,
        },
        'Found string content in user message, should be array (old data)',
      )
      textContent = apiMessage.content
    }

    if (!textContent.trim()) {
      return null
    }

    return {
      id: agentMessage.uuid,
      role: 'user',
      content: textContent,
      timestamp: new Date(agentMessage.timestamp),
      eventId: agentMessage.uuid,
      ...(agentMessage.attachments && { attachments: agentMessage.attachments }),
      ...(agentMessage.contexts && { contexts: agentMessage.contexts }),
      ...(agentMessage.isVisibleInTranscriptOnly !== undefined && {
        isVisibleInTranscriptOnly: agentMessage.isVisibleInTranscriptOnly,
      }),
      ...(agentMessage.isCompactSummary !== undefined && {
        isCompactSummary: agentMessage.isCompactSummary,
      }),
    }
  }

  /**
   * Convert agent event to AgentMessage format for storage
   */
  private convertEventToAgentMessage(event: Event): AgentMessage | null {
    try {
      // Ensure we have the required fields
      if (!(event.payload && event.eventId && event.timestamp)) {
        return null
      }

      if (!isAgentMessageEvent(event)) {
        return null
      }

      // Extract the Claude SDK message from the event payload
      const sdkMessage = event.payload as ClaudeCodeSDKMessage

      // The SDK message should have a 'message' field containing the actual message
      // System init messages don't have a message field - this is expected
      if (!('message' in sdkMessage && sdkMessage.message)) {
        logger.debug(
          {
            eventId: event.eventId,
            sdkMessage,
          },
          'SDK message missing message field',
        )
        return null
      }

      // Determine message type from SDK message structure
      let messageType: 'user' | 'assistant' = 'assistant' // Default
      if (sdkMessage.type === 'user') {
        messageType = 'user'
      } else if (sdkMessage.type === 'assistant') {
        messageType = 'assistant'
      }

      // Create base agent message structure
      const agentSessionId =
        'agentSessionId' in event.payload ? event.payload.agentSessionId : undefined

      // Normalize message content to always be an array
      const normalizedMessage = {
        ...sdkMessage.message,
        content: this.normalizeMessageContent(sdkMessage.message.content),
      }

      const agentMessage: AgentMessage = {
        uuid: event.eventId,
        timestamp: event.timestamp.toISOString(),
        userType: 'user',
        type: messageType,
        message: normalizedMessage, // Store normalized message (content is always array)
        sessionId: agentSessionId as string | undefined,
        // Add optional fields if present in SDK message
        ...('parent_tool_use_id' in sdkMessage &&
          sdkMessage.parent_tool_use_id && {
            parentUuid: sdkMessage.parent_tool_use_id,
          }),
        ...('session_id' in sdkMessage &&
          sdkMessage.session_id && { sessionId: sdkMessage.session_id }),
        // Add visibility fields if present
        ...('isVisibleInTranscriptOnly' in sdkMessage &&
          sdkMessage.isVisibleInTranscriptOnly !== undefined && {
            isVisibleInTranscriptOnly: sdkMessage.isVisibleInTranscriptOnly as boolean,
          }),
        ...('isCompactSummary' in sdkMessage &&
          sdkMessage.isCompactSummary !== undefined && {
            isCompactSummary: sdkMessage.isCompactSummary as boolean,
          }),
      }

      return agentMessage
    } catch (error) {
      logger.warn(
        {
          eventId: event.eventId,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to convert event to agent message',
      )
      return null
    }
  }
}
