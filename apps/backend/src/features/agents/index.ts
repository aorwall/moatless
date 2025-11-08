/**
 * Agents feature exports
 * This module manages AI agent lifecycle, execution, and implementations
 */

// API Routes
export { registerAgentRoutes } from './api/agent.routes'
export { registerClaudeSettingsRoutes } from './api/claude-settings.routes'
export { registerMcpSettingsRoutes } from './api/mcp-settings.routes'
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
export { ClaudeSettingsService } from './services/claude-settings.service'
export { CommandService } from './services/command.service'
export { McpService } from './services/mcp.service'
export { McpSettingsService } from './services/mcp-settings.service'
export { MessageService } from './services/message.service'
export { ToolService } from './services/tool.service'
// DI Symbols
export {
  AgentControllerSymbol,
  ClaudeServiceSymbol,
  ClaudeSettingsServiceSymbol,
  McpSettingsServiceSymbol,
} from './symbols'
