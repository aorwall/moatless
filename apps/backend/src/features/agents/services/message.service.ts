import { randomUUID } from 'node:crypto'
import type {
  AgentEvent,
  AgentMessage,
  AskUserQuestionResponse,
  AssistantAgentMessage,
  AssistantMessage,
  AssistantMessageContent,
  ClaudeCodeSDKMessage,
  Event,
  ExitPlanModeResponse,
  Message,
  MessageContext,
  SystemAgentMessage,
  SystemMessage,
  ToolResult,
  UserAgentMessage,
  UserMessage,
} from '@moatless/types'
import { isTextContent, isToolResultContent, isToolUseContent } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { EventService } from '../../events/services/event.service'
import { FileStorageService } from '../../file-storage'
import { errorTotal } from '../../metrics'
import { TaskMessageRepository } from '../../tasks/db/task-message.repository'

/** Agent messages that carry a transcript message (excludes result messages) */
type TranscriptMessage = UserAgentMessage | AssistantAgentMessage | SystemAgentMessage

// Type guard for agent message events
function isAgentMessageEvent(event: Event): event is AgentEvent & { subType: 'message' } {
  return event.type === 'agent' && event.subType === 'message'
}

@injectable()
export class MessageService {
  private unsubscribe?: (() => void) | undefined

  constructor(
    @inject(TaskMessageRepository) private messageRepository: TaskMessageRepository,
    @inject(FileStorageService) private uploadService: FileStorageService,
    @inject(EventService) private eventService: EventService,
  ) {}

  /**
   * Start listening for message events from EventService.
   * Called once during app initialization.
   * This enables message persistence for both local and remote (Kubernetes) agents.
   */
  startEventSubscription(): void {
    if (this.unsubscribe) {
      logger.warn('MessageService: Event subscription already started')
      return
    }

    this.unsubscribe = this.eventService.subscribeAll(async (event) => {
      if (event.type === 'agent' && event.subType === 'message') {
        try {
          await this.saveAgentMessage(event.taskId, event)
        } catch (error) {
          // Track message persistence failures
          errorTotal
            .labels({
              error_type: 'message_persistence_error',
              service: 'message_service',
              severity: 'warning',
            })
            .inc()

          logger.warn(
            {
              taskId: event.taskId,
              eventId: event.eventId,
              error: error instanceof Error ? error.message : String(error),
            },
            'Failed to persist message event from subscription',
          )
        }
      }
    })
    logger.info('MessageService: Started event subscription for message persistence')
  }

  /**
   * Stop event subscription (for graceful shutdown)
   */
  stopEventSubscription(): void {
    if (this.unsubscribe) {
      this.unsubscribe()
      this.unsubscribe = undefined
      logger.info('MessageService: Stopped event subscription')
    }
  }

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

    // No messages found - return empty array
    logger.debug(
      {
        taskId,
      },
      'No repository transcript found',
    )
    return []
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
   * Get messages that haven't been acknowledged by the agent yet.
   * These are messages where agentSessionId is null.
   * Used for recovery scenarios where we need to resend messages to a new agent session.
   */
  async getUnacknowledgedMessages(taskId: string): Promise<AgentMessage[]> {
    try {
      const messages = await this.messageRepository.getMessagesWithoutAgentSessionId(taskId)
      logger.debug(
        {
          taskId,
          messageCount: messages.length,
        },
        'Retrieved unacknowledged messages',
      )
      return messages
    } catch (error) {
      logger.warn(
        {
          taskId,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to get unacknowledged messages',
      )
      return []
    }
  }

  /**
   * Mark all messages without agentSessionId as received by setting the sessionId.
   * Called when we receive the first user/system event from the agent,
   * indicating it has acknowledged receipt of our messages.
   */
  async markMessagesAsReceivedByAgent(taskId: string, agentSessionId: string): Promise<void> {
    try {
      const count = await this.messageRepository.updateUnacknowledgedMessages(
        taskId,
        agentSessionId,
      )
      if (count > 0) {
        logger.debug(
          { taskId, agentSessionId, messageCount: count },
          'Marked messages as received by agent',
        )
      }
    } catch (error) {
      logger.warn(
        {
          taskId,
          agentSessionId,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to mark messages as received by agent',
      )
    }
  }

  /**
   * Get the latest agent session ID for a task.
   * Used to determine the resume session ID when sending new messages.
   */
  async getLatestAgentSessionId(taskId: string): Promise<string | undefined> {
    try {
      const sessionId = await this.messageRepository.getLatestAgentSessionId(taskId)
      return sessionId ?? undefined
    } catch (error) {
      logger.warn(
        {
          taskId,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to get latest agent session ID',
      )
      return undefined
    }
  }

  /**
   * Save a user message to the message repository
   */
  async saveUserMessage(
    taskId: string,
    message: string,
    sessionId?: string,
    turnNumber?: number,
    fileIds?: string[],
    contexts?: MessageContext[],
    agentMode?: 'edit' | 'plan',
    model?: string,
    resumeSessionId?: string,
  ): Promise<void> {
    try {
      const userMessage: UserAgentMessage = {
        uuid: randomUUID(),
        timestamp: new Date().toISOString(),
        userType: 'user',
        type: 'user',
        message: {
          role: 'user',
          content: [{ type: 'text', text: message }],
        },
        sessionId,
        turnNumber,
        agentMode,
        model,
        resumeSessionId,
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
          turnNumber,
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
          turnNumber,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to persist user message',
      )
      throw error
    }
  }

  /**
   * Save a user input response (AskUserQuestion answer or ExitPlanMode approval)
   * This creates a user message with the response content
   */
  async saveUserInputResponse(
    taskId: string,
    toolName: 'AskUserQuestion' | 'ExitPlanMode',
    response: unknown,
    sessionId?: string,
  ): Promise<void> {
    try {
      // Format the response as readable content
      let content: string
      if (toolName === 'AskUserQuestion') {
        const { answers } = response as { answers: Record<string, string> }
        content = Object.entries(answers)
          .map(([question, answer]) => `**${question}**\n${answer}`)
          .join('\n\n')
      } else {
        const { approved, feedback } = response as { approved: boolean; feedback?: string }
        content = approved
          ? '✓ Plan approved'
          : `✗ Plan rejected${feedback ? `\n\nFeedback: ${feedback}` : ''}`
      }

      const userMessage: AgentMessage = {
        uuid: randomUUID(),
        timestamp: new Date().toISOString(),
        userType: 'user',
        type: 'user',
        message: {
          role: 'user',
          content: [{ type: 'text', text: content }],
        },
        sessionId,
        isVisibleInTranscriptOnly: true,
      }

      await this.messageRepository.append(taskId, userMessage)
      logger.debug(
        {
          taskId,
          toolName,
          messageId: userMessage.uuid,
        },
        'User input response persisted',
      )
    } catch (error) {
      logger.warn(
        {
          taskId,
          toolName,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to persist user input response',
      )
      throw error
    }
  }

  /**
   * Save a user input response as a tool_result message in the message queue.
   * This routes user input (AskUserQuestion/ExitPlanMode) through the same
   * delivery mechanism as regular messages (DB -> notify -> poll).
   */
  async saveUserInputAsToolResult(
    taskId: string,
    toolName: 'AskUserQuestion' | 'ExitPlanMode',
    toolUseId: string,
    response: AskUserQuestionResponse | ExitPlanModeResponse,
  ): Promise<void> {
    try {
      const userMessage: UserAgentMessage = {
        uuid: randomUUID(),
        timestamp: new Date().toISOString(),
        userType: 'user',
        type: 'user',
        message: {
          role: 'user',
          content: [
            {
              type: 'tool_result',
              tool_use_id: toolUseId,
              content: JSON.stringify(response),
              is_error: false,
            },
          ],
        },
      }

      await this.messageRepository.append(taskId, userMessage)
      logger.debug(
        {
          taskId,
          toolName,
          toolUseId,
          messageId: userMessage.uuid,
        },
        'User input response persisted as tool_result message',
      )
    } catch (error) {
      logger.warn(
        {
          taskId,
          toolName,
          toolUseId,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to persist user input as tool_result',
      )
      throw error
    }
  }

  /**
   * Save an agent message from an event to the repository.
   *
   * When we receive a user/system message from the SDK, it means the agent
   * has processed our messages and is now sending back tool results.
   * At this point, we mark all previously unacknowledged messages as received.
   */
  async saveAgentMessage(taskId: string, event: Event): Promise<void> {
    try {
      if (!isAgentMessageEvent(event)) {
        return
      }

      const sdkMessage = event.payload as ClaudeCodeSDKMessage

      // If this is a user or system message from SDK, it means agent received our messages.
      // Mark all unacknowledged messages with this session ID.
      // This happens BEFORE we save the current message.
      if (sdkMessage.type === 'user' || sdkMessage.type === 'system') {
        const agentSessionId =
          'session_id' in sdkMessage ? (sdkMessage.session_id as string | undefined) : undefined
        if (agentSessionId) {
          await this.markMessagesAsReceivedByAgent(taskId, agentSessionId)
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
   * Transform agent messages directly into UI messages (bypassing events)
   */
  private transformAgentMessagesToMessages(agentMessages: AgentMessage[]): Message[] {
    const seenUuids = new Set<string>()
    const toolResultsMap = this.buildToolResultsMap(agentMessages, seenUuids)
    const messages = this.transformMessagesWithToolResults(agentMessages, toolResultsMap, seenUuids)

    // Mark the last assistant message if it has a pending user input request
    this.markPendingUserInput(messages)

    return messages
  }

  /**
   * Check if the last assistant message has a pending user input request
   * (AskUserQuestion or ExitPlanMode tool_use without a tool_result)
   */
  private markPendingUserInput(messages: Message[]): void {
    if (messages.length === 0) {
      return
    }

    const lastMessage = messages[messages.length - 1]
    if (lastMessage?.role !== 'assistant') {
      return
    }

    const assistantMessage = lastMessage as AssistantMessage
    if (!Array.isArray(assistantMessage.content)) {
      return
    }

    // Check if the last content block is a user input tool without a result
    const lastContent = assistantMessage.content[assistantMessage.content.length - 1]
    if (
      lastContent?.type === 'tool_use' &&
      (lastContent.tool_name === 'AskUserQuestion' || lastContent.tool_name === 'ExitPlanMode') &&
      !lastContent.tool_result
    ) {
      assistantMessage.hasPendingUserInput = true
    }
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

      // Check both 'user' and 'system' types for backward compatibility
      if (
        (agentMessage.type === 'user' || agentMessage.type === 'system') &&
        agentMessage.message
      ) {
        this.extractToolResults(agentMessage, toolResultsMap)
      }
    }

    return toolResultsMap
  }

  private extractToolResults(
    agentMessage: TranscriptMessage,
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

    // Handle system messages - show as collapsed entries in the UI
    if (agentMessage.type === 'system' && agentMessage.message) {
      return this.transformAgentSystemMessage(agentMessage)
    }

    if (agentMessage.type === 'user' && agentMessage.message) {
      return this.transformUserMessageIfNotToolResult(agentMessage)
    }

    return null
  }

  private transformUserMessageIfNotToolResult(agentMessage: TranscriptMessage): Message | null {
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
   * Transform a system agent message to a UI SystemMessage
   * System messages are SDK user messages (tool results) that should be collapsed by default
   */
  private transformAgentSystemMessage(agentMessage: TranscriptMessage): SystemMessage | null {
    if (agentMessage.type !== 'system' || !agentMessage.message) {
      return null
    }

    const apiMessage = agentMessage.message

    // Extract text content from system message
    let textContent = ''
    if (apiMessage.content && Array.isArray(apiMessage.content)) {
      textContent = apiMessage.content
        .filter(isTextContent)
        .map((c) => c.text)
        .join('\n')

      // Also extract tool_result content if present
      const toolResults = apiMessage.content.filter(isToolResultContent)
      if (toolResults.length > 0) {
        const resultText = toolResults.map((r) => r.content).join('\n')
        textContent = textContent ? `${textContent}\n${resultText}` : resultText
      }
    } else if (typeof apiMessage.content === 'string') {
      textContent = apiMessage.content
    }

    if (!textContent.trim()) {
      return null
    }

    return {
      id: agentMessage.uuid,
      role: 'system',
      content: textContent,
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

  private transformAgentAssistantMessage(
    agentMessage: TranscriptMessage,
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
    apiMessage: TranscriptMessage['message'],
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

  private handleStringContent(apiMessage: TranscriptMessage['message']): AssistantMessageContent[] {
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
    // Check for both 'user' and 'system' types for backward compatibility
    if (!toolResult || (toolResult.type !== 'user' && toolResult.type !== 'system')) {
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

  private transformAgentUserMessage(agentMessage: TranscriptMessage): UserMessage | null {
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
      // Mark user messages as queued if they haven't been sent to the agent yet
      ...(!agentMessage.agentSessionId && { isQueued: true }),
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
      // SDK user messages (tool results) are converted to 'system' type
      // to distinguish them from initial user messages saved via saveUserMessage
      let messageType: 'user' | 'assistant' | 'system' = 'assistant' // Default
      if (sdkMessage.type === 'user') {
        messageType = 'system' // Changed from 'user' to 'system'
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

      // Ensure timestamp is a Date object (may be string after JSON.parse)
      const timestamp =
        event.timestamp instanceof Date ? event.timestamp : new Date(event.timestamp)

      const agentMessage: AgentMessage = {
        uuid: event.eventId,
        timestamp: timestamp.toISOString(),
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
