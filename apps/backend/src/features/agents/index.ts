/**
 * Agents feature exports
 * This module manages AI agent lifecycle, execution, and implementations
 */

export { ClaudeCodeAgent } from '../claude/agent/claude-code-agent'
export type { AgentOptions, AgentRunOptions } from './agent/base-agent'
// Agent Implementations
export { BaseAgent } from './agent/base-agent'
export { TestAgent } from './agent/test-agent'
// API Routes
export { registerAgentRoutes } from './api/agent.routes'
export { registerMcpSettingsRoutes } from './api/mcp-settings.routes'
export { registerMessagesRoutes } from './api/messages.routes'
// Repositories
export { CommandRepository } from './db/command.repository'
// Services
export { AgentController } from './services/agent-controller.service'
export { CommandService } from './services/command.service'
export { McpService } from './services/mcp.service'
export { McpSettingsService } from './services/mcp-settings.service'
export { MessageService } from './services/message.service'
export { ToolService } from './services/tool.service'
// DI Symbols
export { AgentControllerSymbol, McpSettingsServiceSymbol } from './symbols'
