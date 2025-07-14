import { promises as fs } from 'node:fs'
import type { SDKMessage } from '@anthropic-ai/claude-code'
import type {
  AssistantMessage,
  AssistantMessageContent,
  Message,
  ToolResult,
  UserMessage,
} from '@moatless/types'
import {
  isToolResultContent,
  isToolUseContent,
  type SDKAssistantMessage,
  type SDKUserMessage,
} from '@moatless/types'
import type { ClaudeCodeSettings } from '@moatless/types/types/claude'
import type { Task } from '@moatless/types/types/task'
import { getClaudeEventsFilePath } from '../utils/claude-code-events.js'

export class MessageService {
  private claudeCodeSettings: ClaudeCodeSettings

  constructor(claudeCodeSettings: ClaudeCodeSettings) {
    this.claudeCodeSettings = claudeCodeSettings
  }

  /**
   * Read messages directly from Claude directory using stored settings
   */
  async readMessagesFromClaudeDirectory(task: Task): Promise<Message[]> {
    return this.readMessagesFromClaudeDirectoryWithSettings(this.claudeCodeSettings, task)
  }

  /**
   * Read messages directly from Claude directory (bypassing event service)
   */
  async readMessagesFromClaudeDirectoryWithSettings(
    settings: ClaudeCodeSettings,
    task: Task,
  ): Promise<Message[]> {
    const filePath = getClaudeEventsFilePath(settings, task)
    if (!filePath) {
      console.warn('Cannot read Claude messages - no file path available', {
        taskId: task.taskId,
      })
      return []
    }

    try {
      const fileContent = await fs.readFile(filePath, 'utf-8')
      const lines = fileContent.trim().split('\n').filter(Boolean)

      const sdkMessages: SDKMessage[] = []
      let parseErrors = 0

      for (const line of lines) {
        try {
          const sdkMessage = JSON.parse(line) as SDKMessage
          sdkMessages.push(sdkMessage)
        } catch (parseError) {
          parseErrors++
          console.warn('Failed to parse Claude SDK message', {
            taskId: task.taskId,
            error: parseError instanceof Error ? parseError.message : String(parseError),
          })
        }
      }

      console.debug('Successfully read Claude messages', {
        taskId: task.taskId,
        filePath,
        totalLines: lines.length,
        messagesRead: sdkMessages.length,
        parseErrors,
      })

      return this.transformSDKMessagesToMessages(sdkMessages)
    } catch (error) {
      if (error instanceof Error && 'code' in error && error.code === 'ENOENT') {
        console.debug('Claude messages file does not exist', {
          taskId: task.taskId,
          filePath,
        })
        return []
      }

      console.error('Error reading Claude messages file', error as Error, {
        taskId: task.taskId,
        filePath,
      })
      throw error
    }
  }

  /**
   * Transform SDK messages directly into UI messages (bypassing events)
   */
  private transformSDKMessagesToMessages(sdkMessages: SDKMessage[]): Message[] {
    const messages: Message[] = []
    const toolResultsMap = new Map<string, SDKMessage>()

    // First pass: collect all tool results by tool_use_id
    for (const sdkMessage of sdkMessages) {
      if (this.isSDKUserMessage(sdkMessage)) {
        const apiMessage = sdkMessage.message
        if (apiMessage.content && Array.isArray(apiMessage.content)) {
          for (const content of apiMessage.content) {
            if (isToolResultContent(content)) {
              toolResultsMap.set(content.tool_use_id, sdkMessage)
            }
          }
        }
      }
    }

    // Second pass: transform SDK messages to UI messages
    const processedIds = new Set<string>()

    for (const sdkMessage of sdkMessages) {
      // Skip if already processed as a tool result
      if (processedIds.has(this.getSDKMessageId(sdkMessage))) {
        continue
      }

      if (this.isSDKAssistantMessage(sdkMessage)) {
        const message = this.transformSDKAssistantMessage(sdkMessage, toolResultsMap, processedIds)
        if (message) {
          messages.push(message)
        }
      } else if (this.isSDKUserMessage(sdkMessage)) {
        // Only include user messages that are not tool results
        const apiMessage = sdkMessage.message
        if (apiMessage.content && Array.isArray(apiMessage.content)) {
          const hasToolResult = apiMessage.content.some(isToolResultContent)
          if (!hasToolResult) {
            const message = this.transformSDKUserMessage(sdkMessage)
            if (message) {
              messages.push(message)
            }
          }
        } else {
          const message = this.transformSDKUserMessage(sdkMessage)
          if (message) {
            messages.push(message)
          }
        }
      }
    }

    return messages
  }

  private isSDKAssistantMessage(sdkMessage: SDKMessage): sdkMessage is SDKAssistantMessage {
    return 'message' in sdkMessage && sdkMessage.message?.role === 'assistant'
  }

  private isSDKUserMessage(sdkMessage: SDKMessage): sdkMessage is SDKUserMessage {
    return 'message' in sdkMessage && sdkMessage.message?.role === 'user'
  }

  private getSDKMessageId(sdkMessage: SDKMessage): string {
    const timestamp =
      'timestamp' in sdkMessage && typeof sdkMessage.timestamp === 'number'
        ? sdkMessage.timestamp
        : Date.now()
    return `${timestamp}-${JSON.stringify(sdkMessage).slice(0, 100)}`
  }

  private transformSDKAssistantMessage(
    sdkMessage: SDKAssistantMessage,
    toolResultsMap: Map<string, SDKMessage>,
    processedIds: Set<string>,
  ): AssistantMessage | null {
    const apiMessage = sdkMessage.message
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
          if (toolResult && this.isSDKUserMessage(toolResult)) {
            // Mark the tool result as processed
            processedIds.add(this.getSDKMessageId(toolResult))

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
                  eventId: this.getSDKMessageId(toolResult),
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

    const timestamp =
      'timestamp' in sdkMessage && typeof sdkMessage.timestamp === 'number'
        ? sdkMessage.timestamp
        : Date.now()
    return {
      id: this.getSDKMessageId(sdkMessage),
      role: 'assistant',
      content:
        content.length === 1 && content[0].type === 'text' && content[0].text
          ? content[0].text
          : content,
      timestamp: new Date(timestamp),
      eventId: this.getSDKMessageId(sdkMessage),
    }
  }

  private transformSDKUserMessage(sdkMessage: SDKUserMessage): UserMessage | null {
    const apiMessage = sdkMessage.message

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

    const timestamp =
      'timestamp' in sdkMessage && typeof sdkMessage.timestamp === 'number'
        ? sdkMessage.timestamp
        : Date.now()
    return {
      id: this.getSDKMessageId(sdkMessage),
      role: 'user',
      content: textContent,
      timestamp: new Date(timestamp),
      eventId: this.getSDKMessageId(sdkMessage),
    }
  }
}

export function createMessageService(claudeCodeSettings: ClaudeCodeSettings): MessageService {
  return new MessageService(claudeCodeSettings)
}
