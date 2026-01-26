// Routes

// Re-export FileAttachment from shared package via base-agent
export type { FileAttachment } from '@moatless/sandbox-schemas'
export { registerAgentRoutes } from './api/agent.routes'
export type { AgentEventCallback, AgentOptions, AgentRunRequest } from './base/base-agent'
// Base agent framework
export { BaseAgent } from './base/base-agent'

// Claude Code agent
export { ClaudeCodeAgent } from './claude-code/claude-code-agent'
export { calculateContextTokens } from './claude-code/context-tokens'

// Agent service (centralized agent execution and batching)
export {
  type AgentExecutionResult,
  AgentService,
  type BatchedAgentRequest,
} from './services/agent.service'

// Agent request consumer (Redis Streams)
export {
  AgentRequestConsumer,
  createAgentRequestConsumer,
} from './services/agent-request-consumer'
