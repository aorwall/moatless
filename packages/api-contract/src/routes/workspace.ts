import { createRoute } from '@hono/zod-openapi'
import {
  FileListResponseSchema,
  FileQuerySchema,
  FileSearchResponseSchema,
  GitDiffResponseSchema,
  TaskFileSearchSchema,
  WorkspaceSetupRequestSchema,
  WorkspaceSetupResponseSchema,
  WorkspaceStatusResponseSchema,
} from '@moatless/types'
import { pathParams } from '../params.js'
import { withStandardErrors } from '../responses.js'

export const getWorkspaceStatusRoute = createRoute({
  method: 'get',
  path: '/api/v1/tasks/{taskId}/workspace-status',
  tags: ['Workspace Status'],
  description:
    'Retrieve comprehensive workspace status including environment state, git status, agent state, and capabilities',
  request: { params: pathParams.taskId },
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
    params: pathParams.taskId,
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
    params: pathParams.taskId,
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
  request: { params: pathParams.taskId },
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
    params: pathParams.taskId,
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
