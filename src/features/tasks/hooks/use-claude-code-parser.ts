import { useMemo } from 'react';

import type { AgentEvent } from '@/types/agent-events';

export interface ParsedMessage {
  id: string;
  role: 'assistant' | 'user';
  content: string;
  toolCalls?: Array<ToolCall>;
  toolResults?: Array<ToolResult>;
  usage?: {
    input_tokens?: number;
    output_tokens?: number;
    cache_creation_input_tokens?: number;
    cache_read_input_tokens?: number;
  };
}

export interface ToolCall {
  id: string;
  name: string;
  input: Record<string, unknown>;
}

export interface ToolResult {
  tool_use_id: string;
  content: string | Record<string, unknown>;
  is_error?: boolean;
}

export interface ParsedEvent {
  event_id: string;
  timestamp: string;
  is_error: boolean;
  type:
    | 'message'
    | 'start'
    | 'stop'
    | 'error'
    | 'input'
    | 'result'
    | 'environment'
    | 'other';
  data: ParsedMessage | Record<string, unknown>;
}

const MAX_TEXT_LENGTH = 200;

export const truncateText = (
  text: string,
  maxLength: number = MAX_TEXT_LENGTH,
): { text: string; isTruncated: boolean } => {
  if (text.length <= maxLength) {
    return { text, isTruncated: false };
  }
  return { text: `${text.slice(0, maxLength)}...`, isTruncated: true };
};

const validateMessage = (payload: Record<string, unknown>) => {
  const message = payload.message as Record<string, unknown> | undefined;
  if (!message) {
    return null;
  }

  const content = message.content as Array<Record<string, unknown>> | undefined;
  if (!(content && Array.isArray(content))) {
    return null;
  }

  return { message, content };
};

const createBaseParsedMessage = (
  message: Record<string, unknown>,
): ParsedMessage => ({
  id: (message.id as string) || '',
  role: (message.role as 'assistant' | 'user') || 'assistant',
  content: '',
  toolCalls: [],
  toolResults: [],
  usage: message.usage as ParsedMessage['usage'],
});

const parseTextContent = (item: Record<string, unknown>): string => {
  return (item.text as string) || '';
};

const parseToolUse = (item: Record<string, unknown>): ToolCall => ({
  id: (item.id as string) || '',
  name: (item.name as string) || '',
  input: (item.input as Record<string, unknown>) || {},
});

const parseToolResult = (item: Record<string, unknown>): ToolResult => {
  const content = item.content;
  return {
    tool_use_id: (item.tool_use_id as string) || '',
    content:
      typeof content === 'string' || (content && typeof content === 'object')
        ? (content as string | Record<string, unknown>)
        : 'No content',
    is_error: item.is_error as boolean,
  };
};

const parseContentItems = (
  content: Array<Record<string, unknown>>,
  parsedMessage: ParsedMessage,
): void => {
  for (const item of content) {
    if (item.type === 'text') {
      parsedMessage.content += parseTextContent(item);
    } else if (item.type === 'tool_use') {
      parsedMessage.toolCalls?.push(parseToolUse(item));
    } else if (item.type === 'tool_result') {
      parsedMessage.toolResults?.push(parseToolResult(item));
    }
  }
};

const parseClaudeMessage = (
  payload: Record<string, unknown>,
): ParsedMessage | null => {
  const validation = validateMessage(payload);
  if (!validation) {
    return null;
  }

  const { message, content } = validation;
  const parsedMessage = createBaseParsedMessage(message);

  parseContentItems(content, parsedMessage);

  return parsedMessage;
};

const createBaseEvent = (event: AgentEvent) => ({
  event_id: event.event_id,
  timestamp: event.timestamp,
  is_error: event.is_error,
});

const parseMessageEvent = (event: AgentEvent): ParsedEvent | null => {
  const baseEvent = createBaseEvent(event);

  if (event.type !== 'message' || !event.payload.type) {
    return null;
  }

  const payloadType = event.payload.type as string;
  if (payloadType !== 'assistant' && payloadType !== 'user') {
    return null;
  }

  const parsedMessage = parseClaudeMessage(event.payload);
  if (!parsedMessage) {
    return null;
  }

  return {
    ...baseEvent,
    type: 'message' as const,
    data: parsedMessage,
  };
};

const getEventType = (eventType: string): ParsedEvent['type'] => {
  switch (eventType) {
    case 'start':
      return 'start';
    case 'stop':
      return 'stop';
    case 'error':
      return 'error';
    case 'input':
      return 'input';
    case 'result':
      return 'result';
    case 'environment':
      return 'environment';
    default:
      return 'other';
  }
};

const parseEvent = (event: AgentEvent): ParsedEvent => {
  // First check if this is a message event with Claude format
  if (event.type === 'message' && event.payload.type) {
    const messageEvent = parseMessageEvent(event);
    if (messageEvent) {
      return messageEvent;
    }
  }

  const baseEvent = createBaseEvent(event);
  const type = getEventType(event.type);

  return {
    ...baseEvent,
    type,
    data: event.payload,
  };
};

export interface EventGroup {
  type: 'group';
  inputEvent: ParsedEvent;
  resultEvent: ParsedEvent;
  intermediateEvents: Array<ParsedEvent>;
}

export interface StandaloneEvent {
  type: 'standalone';
  event: ParsedEvent;
}

export type DisplayEvent = EventGroup | StandaloneEvent;

const findNextResultIndex = (
  events: Array<ParsedEvent>,
  startIndex: number,
): number => {
  for (let j = startIndex + 1; j < events.length; j++) {
    if (events[j].type === 'result') {
      return j;
    }
    // Stop if we encounter another input event
    if (events[j].type === 'input') {
      break;
    }
  }
  return -1;
};

const createEventGroup = (
  events: Array<ParsedEvent>,
  inputIndex: number,
  resultIndex: number,
): EventGroup => {
  const intermediateEvents = events
    .slice(inputIndex + 1, resultIndex)
    .filter((e) => e.type !== 'input' && e.type !== 'result');

  return {
    type: 'group',
    inputEvent: events[inputIndex],
    resultEvent: events[resultIndex],
    intermediateEvents,
  };
};

const processInputEvent = (
  events: Array<ParsedEvent>,
  index: number,
): { displayEvent: DisplayEvent; nextIndex: number } => {
  const resultIndex = findNextResultIndex(events, index);

  if (resultIndex !== -1) {
    return {
      displayEvent: createEventGroup(events, index, resultIndex),
      nextIndex: resultIndex + 1,
    };
  }

  return {
    displayEvent: { type: 'standalone', event: events[index] },
    nextIndex: index + 1,
  };
};

export const groupEvents = (
  events: Array<ParsedEvent>,
): Array<DisplayEvent> => {
  const result: Array<DisplayEvent> = [];
  let i = 0;

  while (i < events.length) {
    const event = events[i];

    if (event.type === 'input') {
      const { displayEvent, nextIndex } = processInputEvent(events, i);
      result.push(displayEvent);
      i = nextIndex;
    } else {
      result.push({ type: 'standalone', event });
      i++;
    }
  }

  return result;
};

export const useClaudeCodeParser = (
  events: Array<AgentEvent>,
): Array<ParsedEvent> => {
  return useMemo(() => {
    return events.map(parseEvent);
  }, [events]);
};

export const useEventGroups = (
  events: Array<AgentEvent>,
): Array<DisplayEvent> => {
  return useMemo(() => {
    const parsedEvents = events.map(parseEvent);
    return groupEvents(parsedEvents);
  }, [events]);
};
