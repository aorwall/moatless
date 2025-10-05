import { createRoute } from '@hono/zod-openapi'
import {
  CommandExecutionSchema,
  FileContentResponseSchema,
  FileContentSchema,
  FileListResponseSchema,
  FileQuerySchema,
  FileSearchResponseSchema,
  FileTreeWithChangesResponseSchema,
  GitDiffResponseSchema,
  GitStatusSchema,
  ServerLogsQuerySchema,
  ServerLogsResponseExtendedSchema,
  ShellCommandResultSchema,
  SyncWorkspaceResponseSchema,
  TaskFileSearchSchema,
  WorkspaceRefreshConfigRequestSchema,
  WorkspaceRefreshConfigResponseSchema,
  WorkspaceSetupRequestSchema,
  WorkspaceSetupResponseSchema,
  WorkspaceStatusResponseSchema,
} from '@moatless/types'
import { z } from 'zod'
import { withStandardErrors } from '../responses.js'

const taskIdParams = z.object({ taskId: z.string().min(1) })
const taskIdAndFileParams = z.object({
  taskId: z.string().min(1),
  filePath: z.string().min(1),
})
const baseBranchQuery = z.object({ baseBranch: z.string().optional() })
const pathQuery = z.object({ path: z.string().optional() })

export const getWorkspaceStatusRoute = createRoute({
  method: 'get',
  path: '/api/v1/tasks/{taskId}/workspace-status',
  tags: ['Workspace Status'],
  description:
    'Retrieve comprehensive workspace status including environment state, git status, agent state, and capabilities',
  request: { params: taskIdParams },
  responses: withStandardErrors(
    WorkspaceStatusResponseSchema,
    'Returns complete workspace status including workspace state, environment status, git status, agent info, and capabilities',
  ),
})

export const listWorkspaceFilesRoute = createRoute({
  method: 'get',
  path: '/api/v1/tasks/{taskId}/files',
  tags: ['Workspace Files'],
  description:
    'List files in the workspace directory with optional path filtering and recursive traversal',
  request: {
    params: taskIdParams,
    query: FileQuerySchema,
  },
  responses: withStandardErrors(
    FileListResponseSchema,
    'Returns list of files in the workspace with name, path, and type information',
  ),
})

export const searchWorkspaceFilesRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/{taskId}/search-files',
  tags: ['Workspace Files'],
  description:
    'Search for files in the workspace by filename or path pattern with optional result limit',
  request: {
    params: taskIdParams,
    body: {
      content: {
        'application/json': {
          schema: TaskFileSearchSchema,
        },
      },
    },
  },
  responses: withStandardErrors(
    FileSearchResponseSchema,
    'Returns search results with file paths, match counts, and total matches',
  ),
})

export const getWorkspaceGitDiffRoute = createRoute({
  method: 'get',
  path: '/api/v1/tasks/{taskId}/git-diff',
  tags: ['Workspace Git'],
  description:
    'Retrieve git diff showing all uncommitted changes in the workspace with file list and statistics',
  request: { params: taskIdParams },
  responses: withStandardErrors(
    GitDiffResponseSchema,
    'Returns git diff string, list of changed files, and change statistics (additions, deletions, total changes)',
  ),
})

export const setupWorkspaceRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/{taskId}/setup',
  tags: ['Workspace Operations'],
  description:
    'Initialize and setup the workspace environment including dependency installation and configuration',
  request: {
    params: taskIdParams,
    body: {
      content: {
        'application/json': {
          schema: WorkspaceSetupRequestSchema,
        },
      },
    },
  },
  responses: withStandardErrors(
    WorkspaceSetupResponseSchema,
    'Returns setup completion status with success flag, message, and final workspace status',
  ),
})

export const readWorkspaceFileRoute = createRoute({
  method: 'get',
  path: '/api/v1/tasks/{taskId}/files/{filePath}',
  tags: ['Workspace Files'],
  description:
    'Read file content from workspace with git diff information showing additions and deletions',
  request: {
    params: taskIdAndFileParams,
  },
  responses: withStandardErrors(
    FileContentResponseSchema,
    'Returns file content, encoding, and git change details if file is modified',
  ),
})

export const writeWorkspaceFileRoute = createRoute({
  method: 'put',
  path: '/api/v1/tasks/{taskId}/files/{filePath}',
  tags: ['Workspace Files'],
  description: 'Write or update file content in the workspace',
  request: {
    params: taskIdAndFileParams,
    body: {
      content: {
        'application/json': {
          schema: FileContentSchema,
        },
      },
    },
  },
  responses: withStandardErrors(z.object({ success: z.boolean() }), 'File written successfully'),
})

export const getGitSyncStatusRoute = createRoute({
  method: 'get',
  path: '/api/v1/tasks/{taskId}/git-sync-status',
  tags: ['Workspace Git'],
  description:
    'Get git sync status including current branch, uncommitted changes, conflicts, and commits ahead/behind origin',
  request: { params: taskIdParams },
  responses: withStandardErrors(
    GitStatusSchema,
    'Returns comprehensive git status with branch info, change detection, and sync state',
  ),
})

export const getFileTreeWithChangesRoute = createRoute({
  method: 'get',
  path: '/api/v1/tasks/{taskId}/file-tree-with-changes',
  tags: ['Workspace Files'],
  description: 'Get file tree structure with git change indicators for each file and directory',
  request: {
    params: taskIdParams,
    query: pathQuery,
  },
  responses: withStandardErrors(
    FileTreeWithChangesResponseSchema,
    'Returns hierarchical file tree with git status (modified, added, deleted) for each entry',
  ),
})

export const syncWithMainRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/{taskId}/sync-with-main',
  tags: ['Workspace Git'],
  description: 'Sync workspace with main/base branch by pulling latest changes',
  request: {
    params: taskIdParams,
    query: baseBranchQuery,
  },
  responses: withStandardErrors(
    SyncWorkspaceResponseSchema,
    'Returns sync result with success status, message, and updated branch info',
  ),
})

export const syncWithOriginRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/{taskId}/sync-origin',
  tags: ['Workspace Git'],
  description: 'Fetch latest changes from remote origin repository',
  request: {
    params: taskIdParams,
    query: baseBranchQuery,
  },
  responses: withStandardErrors(
    WorkspaceStatusResponseSchema,
    'Returns updated workspace status after syncing with origin',
  ),
})

export const executeCommandRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/{taskId}/execute',
  tags: ['Workspace Operations'],
  description: 'Execute a shell command in the workspace environment',
  request: {
    params: taskIdParams,
    body: {
      content: {
        'application/json': {
          schema: CommandExecutionSchema,
        },
      },
    },
  },
  responses: withStandardErrors(
    ShellCommandResultSchema,
    'Returns command execution result with stdout, stderr, exit code, and execution metadata',
  ),
})

export const refreshWorkspaceConfigRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/{taskId}/refresh-config',
  tags: ['Workspace Operations'],
  description: 'Refresh workspace configuration and optionally restart development server',
  request: {
    params: taskIdParams,
    body: {
      content: {
        'application/json': {
          schema: WorkspaceRefreshConfigRequestSchema,
        },
      },
    },
  },
  responses: withStandardErrors(
    WorkspaceRefreshConfigResponseSchema,
    'Returns refresh result with success status, message, updated workspace status, and server restart confirmation',
  ),
})

export const getWorkspaceLogsRoute = createRoute({
  method: 'get',
  path: '/api/v1/tasks/{taskId}/workspace/logs',
  tags: ['Workspace Operations'],
  description:
    'Get workspace logs including all shell command outputs, setup logs, and server logs',
  request: {
    params: taskIdParams,
    query: ServerLogsQuerySchema,
  },
  responses: withStandardErrors(
    ServerLogsResponseExtendedSchema,
    'Returns workspace logs with optional line limit and truncation indicator',
  ),
})
