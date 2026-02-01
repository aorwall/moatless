/**
 * @moatless/sandbox-schemas
 *
 * Shared Zod schemas for request/response validation between
 * the main server and sandbox app.
 */

// Agent schemas
export {
  type AgentRunRequest,
  AgentRunRequestSchema,
  type AgentState,
  AgentStateSchema,
  type AgentStatusResponse,
  AgentStatusResponseSchema,
  type ExecuteAgentResponse,
  ExecuteAgentResponseSchema,
  type FileAttachment,
  FileAttachmentSchema,
  type StopAgentResponse,
  StopAgentResponseSchema,
} from './agent.schemas'

// Command schemas
export {
  type CommandRequest,
  CommandRequestSchema,
  type CommandResponse,
  CommandResponseSchema,
  type ShellCommandLogEvent,
  ShellCommandLogEventSchema,
} from './command.schemas'

// File schemas
export {
  type FileMatch,
  FileMatchSchema,
  type FileTreeRequest,
  FileTreeRequestSchema,
  type ListFilesRequest,
  ListFilesRequestSchema,
  type ListFilesResponse,
  ListFilesResponseSchema,
  type ReadFileRequest,
  ReadFileRequestSchema,
  type ReadFileResponse,
  ReadFileResponseSchema,
  type ReadFileWithChangesRequest,
  ReadFileWithChangesRequestSchema,
  type ReadFileWithChangesResponse,
  ReadFileWithChangesResponseSchema,
  type SearchFilesRequest,
  SearchFilesRequestSchema,
  type SearchFilesResponse,
  SearchFilesResponseSchema,
  type WriteFileRequest,
  WriteFileRequestSchema,
  type WriteFileResponse,
  WriteFileResponseSchema,
} from './file.schemas'

// Git schemas
export {
  type CurrentCommitHashResponse,
  CurrentCommitHashResponseSchema,
  type FileTreeNode,
  FileTreeNodeSchema,
  type FileTreeWithChangesResponse,
  FileTreeWithChangesResponseSchema,
  type GitChangesResponse,
  GitChangesResponseSchema,
  type GitDiffFile,
  GitDiffFileSchema,
  type GitDiffResponse,
  GitDiffResponseSchema,
  type GitStatusResponse,
  GitStatusResponseSchema,
  type GitSummary,
  GitSummarySchema,
  type GitSyncStatus,
  GitSyncStatusSchema,
} from './git.schemas'
// Health schemas
export { type HealthResponse, HealthResponseSchema } from './health.schemas'
// Sandbox path constants
export {
  SANDBOX_CACHE_DIR,
  SANDBOX_CLAUDE_CONFIG_DIR,
  SANDBOX_CLAUDE_EXECUTABLE_PATH,
  SANDBOX_DATA_DIR,
  SANDBOX_HOME,
  SANDBOX_PLUGINS_DIR,
  SANDBOX_UPLOAD_DIR,
  SANDBOX_WORKSPACE_DIR,
} from './sandbox-paths'
// Server (preview) schemas
export {
  type ProxyRequest,
  ProxyRequestSchema,
  type ServerLogEvent,
  ServerLogEventSchema,
  type ServerLogsResponse,
  ServerLogsResponseSchema,
  type ServerStatus,
  type ServerStatusResponse,
  ServerStatusResponseSchema,
  ServerStatusSchema,
  type StartServerRequest,
  StartServerRequestSchema,
  type StartServerResponse,
  StartServerResponseSchema,
  type StopServerResponse,
  StopServerResponseSchema,
} from './server.schemas'
// Workspace schemas
export {
  type RepositoryConfig,
  RepositoryConfigSchema,
  type WorkspaceProvisionRequest,
  WorkspaceProvisionRequestSchema,
  type WorkspaceProvisionResponse,
  WorkspaceProvisionResponseSchema,
  type WorkspaceProvisionStatus,
  WorkspaceProvisionStatusSchema,
  type WorkspaceStatusResponse,
  WorkspaceStatusResponseSchema,
} from './workspace.schemas'
