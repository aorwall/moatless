/**
 * Centralized mock utilities for Claude Agent SDK
 *
 * Following the Mock Factory Pattern from docs/patterns/strategic-mocking.md
 * This provides a single source of truth for SDK message mocks, making it easy
 * to update when the SDK changes (like when uuid field was added).
 */

import { randomUUID } from 'node:crypto'
import type {
  Query,
  SDKAssistantMessage,
  SDKMessage,
  SDKResultMessage,
  SDKUserMessage,
} from '@anthropic-ai/claude-agent-sdk'
import type { Mock } from 'vitest'
import { vi } from 'vitest'

/**
 * Create a mock SDK Assistant Message with all required fields
 */
export function createMockSDKAssistantMessage(
  overrides?: Partial<SDKAssistantMessage>,
): SDKAssistantMessage {
  return {
    type: 'assistant',
    uuid: randomUUID(),
    message: {
      id: 'msg_test',
      type: 'message',
      role: 'assistant',
      model: 'claude-sonnet-4-20250514',
      content: [{ type: 'text', text: 'Test message' }],
      stop_reason: 'end_turn',
      stop_sequence: null,
      usage: {
        input_tokens: 10,
        output_tokens: 20,
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: 0,
      },
    },
    parent_tool_use_id: null,
    session_id: 'test-session',
    ...overrides,
  }
}

/**
 * Create a mock SDK User Message with all required fields
 */
export function createMockSDKUserMessage(overrides?: Partial<SDKUserMessage>): SDKUserMessage {
  return {
    type: 'user',
    uuid: randomUUID(),
    message: {
      role: 'user',
      content: [
        {
          type: 'text',
          text: 'Test user message',
        },
      ],
    },
    parent_tool_use_id: null,
    session_id: 'test-session',
    ...overrides,
  }
}

/**
 * Create a mock SDK Result Message with all required fields
 */
export function createMockSDKResultMessage(
  overrides?: Partial<SDKResultMessage>,
): SDKResultMessage {
  return {
    type: 'result',
    uuid: randomUUID(),
    subtype: 'success',
    duration_ms: 1000,
    duration_api_ms: 800,
    is_error: false,
    num_turns: 1,
    result: 'Test result',
    session_id: 'test-session',
    total_cost_usd: 0.001,
    usage: {
      input_tokens: 100,
      output_tokens: 50,
      cache_creation_input_tokens: 0,
      cache_read_input_tokens: 0,
    },
    permission_denials: [],
    ...overrides,
  } as SDKResultMessage
}

/**
 * Create a mock query function that returns an async generator
 * This is the main mock for the Claude Agent SDK query function
 */
export function createMockQuery(messages: SDKMessage[] = [createMockSDKAssistantMessage()]): Query {
  const generator = async function* () {
    for (const message of messages) {
      yield message
    }
  }

  const gen = generator()

  return {
    ...gen,
    [Symbol.asyncIterator]: () => gen,
    interrupt: vi.fn() as Mock,
    setPermissionMode: vi.fn() as Mock,
    setModel: vi.fn() as Mock,
    setMaxThinkingTokens: vi.fn() as Mock,
    supportedCommands: vi.fn() as Mock,
    supportedModels: vi.fn() as Mock,
    mcpServerStatus: vi.fn() as Mock,
    accountInfo: vi.fn().mockResolvedValue({ name: 'test-account' }) as Mock,
  } as Query
}

/**
 * Create a mock query implementation for vi.mock()
 * Use this in vi.mock('@anthropic-ai/claude-agent-sdk')
 */
export function createMockQueryImplementation(
  defaultMessages: SDKMessage[] = [createMockSDKAssistantMessage()],
) {
  return vi.fn().mockImplementation(() => createMockQuery(defaultMessages))
}
