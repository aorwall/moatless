/**
 * Agents feature exports
 * This module manages AI agent lifecycle, execution, and implementations
 */

// API Routes
export { registerAgentRoutes } from './api/agent.routes'
export { registerMessagesRoutes } from './api/messages.routes'
// Repositories
export { CommandRepository } from './db/command.repository'
export type { AgentOptions, AgentRunOptions } from './implementations/base-agent'
// Agent Implementations
export { BaseAgent } from './implementations/base-agent'
export { ClaudeService } from './implementations/claude-code/claude.service'
export { ClaudeCodeAgent } from './implementations/claude-code/claude-code-agent'
export { ProxyAgent } from './implementations/proxy-agent'
export { TestAgent } from './implementations/test-agent'
// Services
export { AgentController } from './services/agent-controller.service'
export { CommandService } from './services/command.service'
export { MessageService } from './services/message.service'
// DI Symbols
export { AgentControllerSymbol, ClaudeServiceSymbol } from './symbols'
