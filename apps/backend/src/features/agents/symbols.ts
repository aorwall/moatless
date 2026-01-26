import type { RequestQueue } from '@moatless/messaging'
import type { AgentRunRequest } from '@moatless/sandbox-schemas'
import type { ServiceIdentifier } from 'inversify'
import type { AgentControlService } from './services/agent-control.service'
import type { AgentRequestBuilder } from './services/agent-request-builder.service'
import type { McpSettingsService } from './services/mcp-settings.service'

/**
 * DI symbol for AgentRequestBuilder.
 * Used to break circular module loading issues when injecting AgentRequestBuilder.
 */
export const AgentRequestBuilderSymbol: ServiceIdentifier<AgentRequestBuilder> =
  Symbol.for('AgentRequestBuilder')

/**
 * DI symbol for AgentControlService.
 * Used to break circular module loading issues when injecting AgentControlService.
 */
export const AgentControlServiceSymbol: ServiceIdentifier<AgentControlService> =
  Symbol.for('AgentControlService')

/**
 * DI symbol for McpSettingsService.
 */
export const McpSettingsServiceSymbol: ServiceIdentifier<McpSettingsService> =
  Symbol.for('McpSettingsService')

/**
 * DI symbol for RequestQueue (agent execution requests).
 * Used to inject the queue implementation (Redis Streams or SQS).
 */
export const RequestQueueSymbol: ServiceIdentifier<RequestQueue<AgentRunRequest>> =
  Symbol.for('RequestQueue')
