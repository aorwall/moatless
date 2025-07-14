import type { Event } from '@moatless/types/types/event'
import { useMemo } from 'react'

export interface ParsedMessage {
  id: string
  role: 'assistant' | 'user'
  content: string
  toolCalls?: Array<ToolCall>
  toolResults?: Array<ToolResult>
  usage?: {
    input_tokens?: number
    output_tokens?: number
    cache_creation_input_tokens?: number
    cache_read_input_tokens?: number
  }
}

export interface ToolCall {
  id: string
  name: string
  input: Record<string, unknown>
}

export interface ToolResult {
  toolUseId: string
  content: string | Record<string, unknown>
  isError?: boolean
}

export interface ParsedEvent {
  eventId: string
  timestamp: string
  isError: boolean
  type: 'message' | 'start' | 'stop' | 'error' | 'result' | 'environment' | 'other'
  data: ParsedMessage | Record<string, unknown>
}

const MAX_TEXT_LENGTH = 200

export const truncateText = (
  text: string,
  maxLength: number = MAX_TEXT_LENGTH,
): { text: string; isTruncated: boolean } => {
  if (text.length <= maxLength) {
    return { text, isTruncated: false }
  }
  return { text: `${text.slice(0, maxLength)}...`, isTruncated: true }
}

const validateMessage = (payload: Record<string, unknown>) => {
  const message = payload.message as Record<string, unknown> | undefined
  if (!message) {
    return null
  }

  const content = message.content as Array<Record<string, unknown>> | undefined
  if (!(content && Array.isArray(content))) {
    return null
  }

  return { message, content }
}

const createBaseParsedMessage = (message: Record<string, unknown>): ParsedMessage => ({
  id: (message.id as string) || '',
  role: (message.role as 'assistant' | 'user') || 'assistant',
  content: '',
  toolCalls: [],
  toolResults: [],
  usage: message.usage as ParsedMessage['usage'],
})

const parseTextContent = (item: Record<string, unknown>): string => {
  return (item.text as string) || ''
}

const parseToolUse = (item: Record<string, unknown>): ToolCall => ({
  id: (item.id as string) || '',
  name: (item.name as string) || '',
  input: (item.input as Record<string, unknown>) || {},
})

const parseToolResult = (item: Record<string, unknown>): ToolResult => {
  const content = item.content
  return {
    toolUseId: (item.toolUseId as string) || '',
    content:
      typeof content === 'string' || (content && typeof content === 'object')
        ? (content as string | Record<string, unknown>)
        : 'No content',
    isError: item.isError as boolean,
  }
}

const parseContentItems = (
  content: Array<Record<string, unknown>>,
  parsedMessage: ParsedMessage,
): void => {
  for (const item of content) {
    if (item.type === 'text') {
      parsedMessage.content += parseTextContent(item)
    } else if (item.type === 'tool_use') {
      parsedMessage.toolCalls?.push(parseToolUse(item))
    } else if (item.type === 'tool_result') {
      parsedMessage.toolResults?.push(parseToolResult(item))
    }
  }
}

const parseClaudeMessage = (payload: Record<string, unknown>): ParsedMessage | null => {
  const validation = validateMessage(payload)
  if (!validation) {
    return null
  }

  const { message, content } = validation
  const parsedMessage = createBaseParsedMessage(message)

  parseContentItems(content, parsedMessage)

  return parsedMessage
}

const createBaseEvent = (event: Event) => ({
  eventId: event.eventId,
  timestamp: event.timestamp ? event.timestamp.toISOString() : new Date().toISOString(),
  isError: event.isError,
})

const parseMessageEvent = (event: Event): ParsedEvent | null => {
  const baseEvent = createBaseEvent(event)

  if (event.type !== 'message' || !event.payload.type) {
    return null
  }

  const payloadType = event.payload.type as string
  if (payloadType !== 'assistant' && payloadType !== 'user') {
    return null
  }

  const parsedMessage = parseClaudeMessage(event.payload)
  if (!parsedMessage) {
    return null
  }

  return {
    ...baseEvent,
    type: 'message' as const,
    data: parsedMessage,
  }
}

const getEventType = (eventType: string): ParsedEvent['type'] => {
  switch (eventType) {
    case 'start':
      return 'start'
    case 'stop':
      return 'stop'
    case 'error':
      return 'error'
    case 'result':
      return 'result'
    case 'environment':
      return 'environment'
    default:
      return 'other'
  }
}

const parseEvent = (event: Event): ParsedEvent => {
  // First check if this is a message event with Claude format
  if (event.type === 'message' && event.payload.type) {
    const messageEvent = parseMessageEvent(event)
    if (messageEvent) {
      return messageEvent
    }
  }

  const baseEvent = createBaseEvent(event)
  const type = getEventType(event.type)

  return {
    ...baseEvent,
    type,
    data: event.payload,
  }
}

export interface EventGroup {
  type: 'group'
  inputEvent: ParsedEvent
  resultEvent: ParsedEvent
  intermediateEvents: Array<ParsedEvent>
}

export interface StandaloneEvent {
  type: 'standalone'
  event: ParsedEvent
}

export type DisplayEvent = EventGroup | StandaloneEvent

export const groupEvents = (events: Array<ParsedEvent>): Array<DisplayEvent> => {
  const result: Array<DisplayEvent> = []
  let i = 0

  while (i < events.length) {
    const event = events[i]

    result.push({ type: 'standalone', event })
    i++
  }

  return result
}

export const useClaudeCodeParser = (events: Array<Event>): Array<ParsedEvent> => {
  return useMemo(() => {
    return events.map(parseEvent)
  }, [events])
}

export const useEventGroups = (events: Array<Event>): Array<DisplayEvent> => {
  return useMemo(() => {
    const parsedEvents = events.map(parseEvent)
    return groupEvents(parsedEvents)
  }, [events])
}
