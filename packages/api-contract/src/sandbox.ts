import { createRoute } from '@hono/zod-openapi'
import {
  CommandExecutionSchema,
  FileSearchResponseSchema,
  FileSearchSchema,
  SandboxSetupRequestSchema,
  SandboxSetupResponseSchema,
  ServerLogsQuerySchema,
  ServerLogsResponseExtendedSchema,
  ShellCommandResultSchema,
} from '@moatless/types'
import { z } from 'zod'

const taskIdParams = z.object({ taskId: z.string().min(1) })

export const searchSandboxFilesRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/{taskId}/search-files',
  tags: ['Sandbox Files'],
  description:
    'Search for files in the sandbox by filename or path pattern with optional result limit',
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

export const setupSandboxRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/{taskId}/setup',
  tags: ['Sandbox Operations'],
  description:
    'Initialize and setup the sandbox environment including dependency installation and configuration',
  request: {
    params: taskIdParams,
    body: {
      content: {
        'application/json': {
          schema: SandboxSetupRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: SandboxSetupResponseSchema,
        },
      },
    },
  },
})

export const executeCommandRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/{taskId}/execute',
  tags: ['Sandbox Operations'],
  description: 'Execute a shell command in the sandbox environment',
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

export const getSandboxLogsRoute = createRoute({
  method: 'get',
  path: '/api/v1/tasks/{taskId}/workspace/logs',
  tags: ['Sandbox Operations'],
  description: 'Get sandbox logs including all shell command outputs, setup logs, and server logs',
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
