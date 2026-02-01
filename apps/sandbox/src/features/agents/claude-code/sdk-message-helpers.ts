import { randomUUID } from 'node:crypto'
import type { SDKUserMessage } from '@anthropic-ai/claude-agent-sdk'
import type { ClaudeCodeSDKMessage } from '@moatless/types'
import type { Logger } from 'pino'
import type { AgentRunRequest } from '../base/base-agent'

// Anthropic SDK message content types (matching the API format)
export type MessageContentBlock =
  | { type: 'text'; text: string }
  | {
      type: 'image'
      source: {
        type: 'base64'
        media_type: 'image/png' | 'image/jpeg' | 'image/gif' | 'image/webp'
        data: string
      }
    }

/**
 * Extract text message from content array.
 * Combines all text content blocks into a single message.
 */
export function extractMessage(request: AgentRunRequest): string {
  if (!request.content || request.content.length === 0) {
    return ''
  }
  return request.content
    .filter((c): c is { type: 'text'; text: string } => c.type === 'text')
    .map((c) => c.text)
    .join('\n')
}

/**
 * Create an async generator that yields one SDKUserMessage per request.
 *
 * Each message shares the same session_id, allowing Claude to process them
 * sequentially within a single SDK query call. Works for both single
 * requests and batches.
 */
// biome-ignore lint/suspicious/useAwait: Generator yields synchronously but must be async for SDK compatibility
export async function* createSDKMessageStream(
  requests: AgentRunRequest[],
  sessionId: string,
): AsyncGenerator<SDKUserMessage> {
  for (const request of requests) {
    const message = extractMessage(request)
    const nonTextContentBlocks = request.content.filter((c) => c.type !== 'text')

    const messageBlocks: MessageContentBlock[] = [{ type: 'text', text: message }]
    for (const block of nonTextContentBlocks) {
      messageBlocks.push(block as MessageContentBlock)
    }

    const sdkMessage: SDKUserMessage = {
      type: 'user',
      message: {
        role: 'user',
        content: messageBlocks,
        // biome-ignore lint/suspicious/noExplicitAny: APIUserMessage type from @anthropic-ai/sdk not directly importable
      } as any,
      parent_tool_use_id: null,
      uuid: randomUUID() as `${string}-${string}-${string}-${string}-${string}`,
      session_id: sessionId,
    }

    yield sdkMessage
  }
}

/** Timeout in ms for receiving the first SDK message */
export const FIRST_MESSAGE_TIMEOUT_MS = 30_000

/**
 * Wrap an async iterable with a timeout for the first yielded value.
 * If no value is yielded within the timeout, the abort controller is triggered
 * and a diagnostic error is thrown.
 */
export async function* withFirstMessageTimeout<T>(
  iterable: AsyncIterable<T>,
  abortController: AbortController,
  agentLogger: Logger,
  stderrOutput: string[],
  lastProcessExitInfo: { code: number | null; signal: string | null } | null,
): AsyncGenerator<T> {
  let receivedFirst = false
  const timer = setTimeout(() => {
    if (!receivedFirst) {
      agentLogger.error(
        {
          timeoutMs: FIRST_MESSAGE_TIMEOUT_MS,
          stderrOutput: stderrOutput.join('\n') || 'none',
        },
        'Timed out waiting for first SDK message',
      )
      abortController.abort()
    }
  }, FIRST_MESSAGE_TIMEOUT_MS)

  try {
    for await (const value of iterable) {
      if (!receivedFirst) {
        receivedFirst = true
        clearTimeout(timer)
      }
      yield value
    }
    // If we never received a message, clear and throw
    if (!receivedFirst) {
      clearTimeout(timer)
      throw new Error(
        `No SDK messages received. Process exit: ${JSON.stringify(lastProcessExitInfo)}. stderr: ${stderrOutput.join('\n') || 'none'}`,
      )
    }
  } catch (error) {
    clearTimeout(timer)
    if (!receivedFirst && error instanceof Error && error.name === 'AbortError') {
      throw new Error(
        `Timed out after ${FIRST_MESSAGE_TIMEOUT_MS}ms waiting for first SDK message. ` +
          `Process exit: ${JSON.stringify(lastProcessExitInfo)}. ` +
          `stderr: ${stderrOutput.join('\n') || 'none'}`,
      )
    }
    throw error
  }
}

/**
 * Get log-friendly info string from an SDK message.
 */
export function getSDKMessageLogInfo(sdkMessage: ClaudeCodeSDKMessage): string {
  let detailedInfo = ''

  // Check if this message has content (assistant/user messages have message.content)
  if (
    'message' in sdkMessage &&
    sdkMessage.message &&
    'content' in sdkMessage.message &&
    sdkMessage.message.content
  ) {
    const content = Array.isArray(sdkMessage.message.content)
      ? sdkMessage.message.content
      : [sdkMessage.message.content]
    const contentTypes = content
      .map((item: unknown) => {
        if (!(item && typeof item === 'object' && 'type' in item)) {
          return 'unknown'
        }

        if (item.type === 'tool_use' && 'name' in item) {
          return `tool_use:${item.name}`
        }

        return item.type as string
      })
      .join(', ')
    detailedInfo += `, content: [${contentTypes}]`
  }

  // Add result info if it's a result message
  if (sdkMessage.type === 'result') {
    const subtype = 'subtype' in sdkMessage ? sdkMessage.subtype : 'unknown'
    const isError = 'is_error' in sdkMessage ? sdkMessage.is_error : false
    detailedInfo += `, subtype: ${subtype}, error: ${isError}`
  }

  return detailedInfo
}

/**
 * Log warning for error content items in SDK messages.
 */
export function logErrorContentItems(
  agentLogger: Logger,
  sdkMessage: ClaudeCodeSDKMessage,
  eventCount: number,
): void {
  if (
    !(
      'message' in sdkMessage &&
      sdkMessage.message &&
      'content' in sdkMessage.message &&
      sdkMessage.message.content
    )
  ) {
    return
  }

  const content = Array.isArray(sdkMessage.message.content)
    ? sdkMessage.message.content
    : [sdkMessage.message.content]

  for (const contentItem of content) {
    if (
      contentItem &&
      typeof contentItem === 'object' &&
      'is_error' in contentItem &&
      contentItem.is_error === true
    ) {
      const contentType = 'type' in contentItem ? (contentItem.type as string) : 'unknown'
      const errorContent =
        'content' in contentItem ? (contentItem.content as string) : 'No error message'
      const toolUseId =
        'tool_use_id' in contentItem ? (contentItem.tool_use_id as string) : undefined

      agentLogger.warn(
        {
          eventCount,
          contentType,
          errorContent,
          toolUseId,
          fullContentItem: contentItem,
        },
        `SDK message contains error content: ${contentType} error`,
      )
    }
  }
}
