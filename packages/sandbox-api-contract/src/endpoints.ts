/**
 * Sandbox API endpoint paths
 *
 * These are the HTTP endpoints exposed by the sandbox app
 * for communication with the main server.
 */

export const SANDBOX_ENDPOINTS = {
  // Health
  HEALTH: '/health',

  // Agent
  EXECUTE_AGENT: '/execute-agent',
  STOP_AGENT: '/stop-agent',
  AGENT_STATUS: '/agent-status',

  // Command
  COMMAND: '/command',

  // File operations
  READ_FILE: '/read-file',
  WRITE_FILE: '/write-file',
  LIST_FILES: '/list-files',
  SEARCH_FILES: '/search-files',
  READ_FILE_WITH_CHANGES: '/read-file-with-changes',
  FILE_TREE_WITH_CHANGES: '/file-tree-with-changes',

  // Git operations
  GIT_STATUS: '/git-status',
  GIT_CHANGES: '/git-changes',
  GIT_DIFF: '/git-diff',
  CURRENT_COMMIT_HASH: '/current-commit-hash',

  // Preview server
  SERVER_START: '/server/start',
  SERVER_STOP: '/server/stop',
  SERVER_STATUS: '/server/status',
  SERVER_LOGS: '/server/logs',
  SERVER_PROXY: '/server/proxy',

  // Workspace
  WORKSPACE_PROVISION: '/workspace/provision',
  WORKSPACE_STATUS: '/workspace/status',
} as const

export type SandboxEndpoint = (typeof SANDBOX_ENDPOINTS)[keyof typeof SANDBOX_ENDPOINTS]
