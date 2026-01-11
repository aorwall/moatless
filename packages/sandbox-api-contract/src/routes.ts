/**
 * Re-export all sandbox API routes
 */

// Agent routes
export { agentStatusRoute, executeAgentRoute, stopAgentRoute } from './routes/agent.routes'

// Command routes
export { executeCommandRoute } from './routes/command.routes'

// File routes
export {
  fileTreeWithChangesRoute,
  listFilesRoute,
  readFileRoute,
  readFileWithChangesRoute,
  searchFilesRoute,
  writeFileRoute,
} from './routes/file.routes'

// Git routes
export {
  currentCommitHashRoute,
  gitChangesRoute,
  gitDiffRoute,
  gitStatusRoute,
} from './routes/git.routes'
// Health routes
export { healthRoute } from './routes/health.routes'
// Server routes
export {
  serverLogsRoute,
  serverStatusRoute,
  startServerRoute,
  stopServerRoute,
} from './routes/server.routes'
// Workspace routes
export { workspaceProvisionRoute, workspaceStatusRoute } from './routes/workspace.routes'
