import { createRoute } from '@hono/zod-openapi'
import {
  CommandExecutionSchema,
  FileSearchResponseSchema,
  FileSearchSchema,
  ServerLogsQuerySchema,
  ServerLogsResponseExtendedSchema,
  ShellCommandResultSchema,
  WorkspaceSetupRequestSchema,
  WorkspaceSetupResponseSchema,
  WorkspaceStatusResponseSchema,
} from '@moatless/types'
import { z } from 'zod'

const taskIdParams = z.object({ taskId: z.string().min(1) })

export const getWorkspaceStatusRoute = createRoute({
  method: 'get',
  path: '/api/v1/tasks/{taskId}/workspace-status',
  tags: ['Workspace Status'],
  description:
    'Retrieve comprehensive workspace status including environment state, agent state, and capabilities',
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
