/**
 * Agents feature exports
 * This module manages agent execution and related services.
 * Note: Agent implementations (ClaudeCodeAgent, etc.) live in the sandbox package.
 */

// Re-export types from shared package
export type { AgentRunRequest, FileAttachment } from '@moatless/sandbox-schemas'

// API Routes
export { registerMessagesRoutes } from './api/messages.routes'
// Repositories
export { CommandRepository } from './db/command.repository'
// Services
export { AgentControlService } from './services/agent-control.service'
export { AgentRequestService } from './services/agent-request.service'
export {
  AgentRequestBuilder,
  type AgentRequestBuildOptions,
} from './services/agent-request-builder.service'
export { CommandService } from './services/command.service'
export { MessageService } from './services/message.service'
// DI Symbols
export { AgentControlServiceSymbol, AgentRequestBuilderSymbol, RequestQueueSymbol } from './symbols'
