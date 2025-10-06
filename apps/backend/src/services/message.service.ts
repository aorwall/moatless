import { randomUUID } from 'node:crypto'
import type {
  AgentMessage,
  AssistantMessage,
  AssistantMessageContent,
  Event,
  Message,
  ToolResult,
  UserMessage,
} from '@moatless/types'
import { isToolResultContent, isToolUseContent } from '@moatless/types'
import type { Task, TaskIteration } from '@moatless/types/types/task'
import type { ClaudeService } from '../agents/claude-code/claude.service.js'
import { logger } from '../utils/logger.js'
import type { StorageBackend } from './storage.js'
import type { TaskService } from './task.service.js'

export class MessageService {
  constructor(
    private claudeService: ClaudeService,
    private taskService: TaskService,
    private storage: StorageBackend,
  ) {}

  /**
   * Get task messages by task ID
   * This is the main public method that should be used by routes
   * Returns AgentMessages directly for API consumption
   */
  async getTaskMessages(taskId: string): Promise<AgentMessage[]> {
    const task = await this.taskService.getTask(taskId)

    if (!task) {
      throw new Error(`Task not found: ${taskId}`)
    }

    // Only support Claude Code tasks
    if (task.agentType !== 'claude-code') {
      throw new Error('Only Claude Code tasks are supported for message reading')
    }

    // Return AgentMessages directly from storage
    return this.getMessagesFromStorage(taskId)
  }

  /**
   * Get messages from local JSONL storage
   */
  async getMessagesFromStorage(taskId: string): Promise<AgentMessage[]> {
    try {
      const messages = await this.storage.readJsonl<AgentMessage>(`messages/${taskId}`)
      logger.debug('Read messages from local storage', {
        taskId,
        messageCount: messages.length,
      })
      return messages
    } catch (error) {
      logger.warn('Failed to read messages from local storage', {
        taskId,
        error: error instanceof Error ? error.message : String(error),
      })
      return []
    }
  }

  /**
   * Save a user message to local storage
   */
  async saveUserMessage(taskId: string, message: string, sessionId?: string): Promise<void> {
    try {
      const userMessage: AgentMessage = {
        uuid: randomUUID(),
        timestamp: new Date().toISOString(),
        userType: 'user',
        type: 'user',
        message: {
          role: 'user',
          content: message,
        },
        sessionId,
      }

      await this.storage.appendJsonl(`messages/${taskId}`, userMessage)
      logger.debug('User message persisted', {
        taskId,
        messageId: userMessage.uuid,
        hasSessionId: !!sessionId,
      })
    } catch (error) {
      logger.warn('Failed to persist user message', {
        taskId,
        error: error instanceof Error ? error.message : String(error),
      })
      throw error
    }
  }

  /**
   * Save an agent message from an event to local storage
   */
  async saveAgentMessage(taskId: string, event: Event): Promise<void> {
    try {
      if (event.type === 'agent' && event.subType === 'message' && event.payload) {
        const sdkMessage = event.payload as any

        // Check if this is a duplicate initial message (only for user messages)
        if (sdkMessage.type === 'user') {
          const isDuplicate = await this.isInitialMessageDuplicate(taskId, event, sdkMessage)
          if (isDuplicate) {
            return
          }
        }

        const agentMessage = this.convertEventToAgentMessage(event)
        if (agentMessage) {
          await this.storage.appendJsonl(`messages/${taskId}`, agentMessage)
          logger.debug('Agent message persisted', {
            taskId,
            messageType: agentMessage.type,
            timestamp: agentMessage.timestamp,
          })
        }
      }
    } catch (error) {
      logger.warn('Failed to persist agent message', {
        taskId,
        eventType: event.type,
        eventSubType: event.subType,
        error: error instanceof Error ? error.message : String(error),
      })
      throw error
    }
  }

  /**
   * Check if a user message is a duplicate of the iteration's initial message
   */
  private async isInitialMessageDuplicate(
    taskId: string,
    event: Event,
    sdkMessage: any,
  ): Promise<boolean> {
    const iterationId =
      event.type === 'agent' && 'iterationId' in event ? event.iterationId : undefined
    if (!(iterationId && sdkMessage.message?.content)) {
      return false
    }

    const task = await this.storage.getTask(taskId)
    if (!task) {
      return false
    }

    const iteration = task.iterations.find((it) => it.iterationNumber === iterationId)
    if (!iteration?.initialMessage) {
      return false
    }

    const messageContent =
      typeof sdkMessage.message.content === 'string'
        ? sdkMessage.message.content
        : Array.isArray(sdkMessage.message.content)
          ? sdkMessage.message.content
              .filter((c: any) => c.type === 'text')
              .map((c: any) => c.text)
              .join('\n')
          : ''

    if (messageContent === iteration.initialMessage) {
      logger.debug('Skipping duplicate initial message', {
        taskId,
        iterationNumber: iteration.iterationNumber,
      })
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
      const allAgentMessages: AgentMessage[] = []

      // Iterate over all task iterations and read messages for each session
      for (const iteration of task.iterations) {
        if (!iteration.agentSessionId) {
          logger.debug('No agent session ID for iteration', {
            taskId: task.taskId,
            iterationNumber: iteration.iterationNumber,
          })
          continue
        }

        // Read messages for this specific session
        const agentMessages = await this.claudeService.readClaudeMessages(iteration.agentSessionId)

        // If no messages found but iteration has data, create messages from iteration
        if (agentMessages.length === 0) {
          // Add initial message if exists
          if (iteration.initialMessage) {
            const userMessage: AgentMessage = {
              userType: 'user',
              uuid: crypto.randomUUID(),
              timestamp: iteration.startTime.toISOString(),
              type: 'user',
              message: {
                role: 'user',
                content: iteration.initialMessage,
              },
            }
            allAgentMessages.push(userMessage)
          }

          // Add result as assistant message if exists
          if (iteration.result) {
            const resultContent = this.parseMessageFromIteration(iteration)
            if (resultContent) {
              const assistantMessage: AgentMessage = {
                userType: 'user',
                uuid: crypto.randomUUID(),
                timestamp: (iteration.endTime || iteration.startTime).toISOString(),
                type: 'assistant',
                message: {
                  type: 'message',
                  role: 'assistant',
                  content: [
                    {
                      type: 'text',
                      text: resultContent,
                    },
                  ],
                },
              }
              allAgentMessages.push(assistantMessage)
            }
          }
        } else {
          // Add the found messages, handling initial message properly
          const messagesToAdd: AgentMessage[] = []

          if (iteration.initialMessage) {
            // Always add the iteration's initial message first
            logger.debug('Adding iteration initial message', {
              taskId: task.taskId,
              iterationNumber: iteration.iterationNumber,
              agentSessionId: iteration.agentSessionId,
            })

            const initialUserMessage: AgentMessage = {
              userType: 'user',
              uuid: crypto.randomUUID(),
              timestamp: iteration.startTime.toISOString(),
              type: 'user',
              message: {
                role: 'user',
                content: iteration.initialMessage,
              },
            }
            messagesToAdd.push(initialUserMessage)

            // Find the first user message in Claude messages to skip (to avoid duplication)
            let firstUserMessageIndex = -1
            for (let i = 0; i < agentMessages.length; i++) {
              const agentMsg = agentMessages[i]
              if (agentMsg && agentMsg.type === 'user') {
                // Check if this is a real user message (not a tool result)
                const msg = agentMsg
                if (msg.message && msg.message.content) {
                  const content = Array.isArray(msg.message.content)
                    ? msg.message.content
                    : [msg.message.content]
                  // Check if this has actual text content (not just tool results)
                  const hasTextContent = content.some(
                    (c: any) => typeof c === 'string' || (c && c.type === 'text'),
                  )
                  if (hasTextContent) {
                    firstUserMessageIndex = i
                    logger.debug('Found first user message to skip at index', {
                      index: i,
                      taskId: task.taskId,
                      iterationNumber: iteration.iterationNumber,
                    })
                    break
                  }
                }
              }
            }

            // Add all messages except the first user message (which we replaced)
            for (let i = 0; i < agentMessages.length; i++) {
              if (i !== firstUserMessageIndex) {
                const message = agentMessages[i]
                if (message) {
                  messagesToAdd.push(message)
                }
              }
            }
          } else {
            // No initial message, just add all Claude messages
            messagesToAdd.push(...agentMessages)
          }

          allAgentMessages.push(...messagesToAdd)
        }
      }

      // If we got messages from Claude files, transform and return them
      if (allAgentMessages.length > 0) {
        logger.debug('Found Claude messages', {
          taskId: task.taskId,
          messageCount: allAgentMessages.length,
        })
        return this.transformAgentMessagesToMessages(allAgentMessages)
      }

      // Fall back to creating messages from task iterations if no Claude files
      logger.info('No Claude messages found, falling back to task iterations', {
        taskId: task.taskId,
      })
      return this.createMessagesFromTaskIterations(task)
    } catch (error) {
      logger.error('Error reading Claude messages', error as Error, {
        taskId: task.taskId,
      })
      // Fall back to task iterations on error
      return this.createMessagesFromTaskIterations(task)
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
    const messages: Message[] = []
    const toolResultsMap = new Map<string, AgentMessage>()
    const seenUuids = new Set<string>()

    // First pass: collect all tool results by tool_use_id and filter duplicates
    for (const agentMessage of agentMessages) {
      // Skip messages with duplicate UUIDs
      if (seenUuids.has(agentMessage.uuid)) {
        continue
      }
      seenUuids.add(agentMessage.uuid)

      if (agentMessage.type === 'user' && agentMessage.message) {
        const apiMessage = agentMessage.message
        if (apiMessage.content && Array.isArray(apiMessage.content)) {
          for (const content of apiMessage.content) {
            if (isToolResultContent(content)) {
              toolResultsMap.set(content.tool_use_id, agentMessage)
            }
          }
        }
      }
    }

    // Second pass: transform agent messages to UI messages
    const processedIds = new Set<string>()

    for (const agentMessage of agentMessages) {
      // Skip if already processed as a tool result or is a duplicate UUID
      if (processedIds.has(agentMessage.uuid) || !seenUuids.has(agentMessage.uuid)) {
        continue
      }

      if (agentMessage.type === 'assistant' && agentMessage.message) {
        const message = this.transformAgentAssistantMessage(
          agentMessage,
          toolResultsMap,
          processedIds,
        )
        if (message) {
          messages.push(message)
        }
      } else if (agentMessage.type === 'user' && agentMessage.message) {
        // Only include user messages that are not tool results
        const apiMessage = agentMessage.message
        if (apiMessage.content && Array.isArray(apiMessage.content)) {
          const hasToolResult = apiMessage.content.some(isToolResultContent)
          if (!hasToolResult) {
            const message = this.transformAgentUserMessage(agentMessage)
            if (message) {
              messages.push(message)
            }
          }
        } else {
          const message = this.transformAgentUserMessage(agentMessage)
          if (message) {
            messages.push(message)
          }
        }
      }
    }

    return messages
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

    logger.debug('Created messages from task iterations', {
      taskId: task.taskId,
      totalIterations: task.iterations.length,
      messagesCreated: messages.length,
    })

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
        // Look for common message content fields
        if (typeof parsed.content === 'string') {
          return parsed.content
        }
        if (typeof parsed.message === 'string') {
          return parsed.message
        }
        if (typeof parsed.text === 'string') {
          return parsed.text
        }
        // If it's an array, try to extract text content
        if (Array.isArray(parsed.content)) {
          const textContent = parsed.content
            .filter((item: any) => item && typeof item === 'object' && item.type === 'text')
            .map((item: any) => item.text)
            .filter((text: any) => typeof text === 'string')
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
    const content: AssistantMessageContent[] = []

    if (apiMessage.content && Array.isArray(apiMessage.content)) {
      for (const messageContent of apiMessage.content) {
        if (messageContent.type === 'text') {
          content.push({
            type: 'text',
            text: messageContent.text,
          })
        } else if (isToolUseContent(messageContent)) {
          const toolUseId = messageContent.id
          const toolResult = toolResultsMap.get(toolUseId)

          let toolResultContent: ToolResult | undefined
          if (toolResult && toolResult.type === 'user') {
            // Mark the tool result as processed
            processedIds.add(toolResult.uuid)

            // Extract tool result content
            const toolResultAPI = toolResult.message
            if (toolResultAPI.content && Array.isArray(toolResultAPI.content)) {
              const resultContent = toolResultAPI.content.find(isToolResultContent)
              if (resultContent) {
                toolResultContent = {
                  content: Array.isArray(resultContent.content)
                    ? resultContent.content
                        .map((c: any) => (c.type === 'text' ? c.text : JSON.stringify(c)))
                        .join('\n')
                    : typeof resultContent.content === 'string'
                      ? resultContent.content
                      : JSON.stringify(resultContent.content),
                  is_error: resultContent.is_error,
                  eventId: toolResult.uuid,
                }
              }
            }
          }

          content.push({
            type: 'tool_use',
            tool_name: messageContent.name,
            tool_input: messageContent.input,
            tool_use_id: toolUseId,
            tool_result: toolResultContent,
          })
        }
      }
    }

    return {
      id: agentMessage.uuid,
      role: 'assistant',
      content: content.length > 0 ? content : [],
      timestamp: new Date(agentMessage.timestamp),
      eventId: agentMessage.uuid,
    }
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
        .filter((c: any) => c.type === 'text')
        .map((c: any) => c.text)
        .join('\n')
    } else if (typeof apiMessage.content === 'string') {
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

      // Extract the Claude SDK message from the event payload
      const sdkMessage = event.payload as any

      // The SDK message should have a 'message' field containing the actual message
      if (!sdkMessage.message) {
        logger.warn('SDK message missing message field', { eventId: event.eventId })
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
      const agentSessionId = event.type === 'agent' ? event.payload?.agentSessionId : undefined
      const agentMessage: AgentMessage = {
        uuid: event.eventId,
        timestamp: event.timestamp.toISOString(),
        userType: 'user',
        type: messageType,
        message: sdkMessage.message, // Store the message field from the SDK message
        sessionId: agentSessionId,
        // Add optional fields if present in SDK message
        ...(sdkMessage.parent_tool_use_id && { parentUuid: sdkMessage.parent_tool_use_id }),
        ...(sdkMessage.session_id && { sessionId: sdkMessage.session_id }),
      }

      return agentMessage
    } catch (error) {
      logger.warn('Failed to convert event to agent message', {
        eventId: event.eventId,
        error: error instanceof Error ? error.message : String(error),
      })
      return null
    }
  }
}

export function createMessageService(
  claudeService: ClaudeService,
  taskService: TaskService,
  storage: StorageBackend,
): MessageService {
  return new MessageService(claudeService, taskService, storage)
}
