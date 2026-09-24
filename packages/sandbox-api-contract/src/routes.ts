/**
 * Re-export all sandbox API routes
 */

// Agent routes
export { agentStatusRoute, executeAgentRoute, stopAgentRoute } from './routes/agent.routes'

// Command routes
export { executeCommandRoute } from './routes/command.routes'

// File routes
export { searchFilesRoute } from './routes/file.routes'

// Health routes
export { healthRoute } from './routes/health.routes'
// Sandbox routes
export { sandboxProvisionRoute, sandboxStatusRoute } from './routes/sandbox.routes'
