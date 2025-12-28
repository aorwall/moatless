import { createRoute } from '@hono/zod-openapi'
import {
  CommandExecutionSchema,
  FileContentResponseSchema,
  FileContentSchema,
  FileListResponseSchema,
  FileQuerySchema,
  FileSearchResponseSchema,
  FileSearchSchema,
  FileTreeWithChangesResponseSchema,
  GitDiffResponseSchema,
  ServerLogsQuerySchema,
  ServerLogsResponseExtendedSchema,
  ShellCommandResultSchema,
  WorkspaceSetupRequestSchema,
  WorkspaceSetupResponseSchema,
  WorkspaceStatusResponseSchema,
} from '@moatless/types'
import { z } from 'zod'

const taskIdParams = z.object({ taskId: z.string().min(1) })
const taskIdAndFileParams = z.object({
  taskId: z.string().min(1),
  filePath: z.string().min(1),
})
const pathQuery = z.object({ path: z.string().optional() })

export const getWorkspaceStatusRoute = createRoute({
  method: 'get',
  path: '/api/v1/tasks/{taskId}/workspace-status',
  tags: ['Workspace Status'],
  description:
    'Retrieve comprehensive workspace status including environment state, git status, agent state, and capabilities',
  request: { params: taskIdParams },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: WorkspaceStatusResponseSchema,
        },
      },
    },
  },
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
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: FileListResponseSchema,
        },
      },
    },
  },
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
          schema: FileSearchSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: FileSearchResponseSchema,
        },
      },
    },
  },
})

export const getWorkspaceGitDiffRoute = createRoute({
  method: 'get',
  path: '/api/v1/tasks/{taskId}/git-diff',
  tags: ['Workspace Git'],
  description:
    'Retrieve git diff showing all uncommitted changes in the workspace with file list and statistics',
  request: { params: taskIdParams },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: GitDiffResponseSchema,
        },
      },
    },
  },
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
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: WorkspaceSetupResponseSchema,
        },
      },
    },
  },
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
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: FileContentResponseSchema,
        },
      },
    },
  },
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
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: z.object({ success: z.boolean() }),
        },
      },
    },
  },
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
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: FileTreeWithChangesResponseSchema,
        },
      },
    },
  },
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
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: ShellCommandResultSchema,
        },
      },
    },
  },
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
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: ServerLogsResponseExtendedSchema,
        },
      },
    },
  },
})

export const retryWorkspaceRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/{taskId}/workspace-retry',
  tags: ['Workspace Operations'],
  description:
    'Retry workspace provisioning after a failure. Clears the failed state and triggers re-provisioning.',
  request: {
    params: taskIdParams,
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: z.object({
            success: z.boolean(),
            message: z.string(),
          }),
        },
      },
    },
  },
})
