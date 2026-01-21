import { createRoute } from '@hono/zod-openapi'
import {
  CloseTaskResponseSchemaAPI,
  CreateTaskRequestSchema,
  CreateVibeTaskRequestSchema,
  DeleteTaskResponseSchema,
  DeployResponseSchema,
  McpResponseSchema,
  MessagesResponseSchema,
  SendMessageRequestSchema,
  SendMessageResponseSchema,
  SimpleTaskResponseSchema,
  StartTaskResponseSchema,
  StopAgentResponseSchema,
  TaskCommandSchema,
  TaskCommandsResponseSchema,
  TaskListQuerySchema,
  TaskListResponseSchema,
  TaskResponseSchema,
  UserInputResponseRequestSchema,
} from '@moatless/types'
import { z } from 'zod'
import { pathParams } from './params.js'

// ============================================================================
// Task Routes
// ============================================================================

/**
 * POST /api/v1/tasks
 * Create a new AI agent task
 */
export const createTaskRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks',
  tags: ['Tasks'],
  description: 'Create a new AI agent task to make code changes in a repository',
  request: {
    body: {
      content: {
        'application/json': {
          schema: CreateTaskRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: SimpleTaskResponseSchema,
        },
      },
    },
  },
})

/**
 * POST /api/v1/tasks/vibe
 * Create a new vibe task from template
 */
export const createVibeTaskRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/vibe',
  tags: ['Tasks'],
  description: 'Create a new vibe task from a predefined template for quick prototyping',
  request: {
    body: {
      content: {
        'application/json': {
          schema: CreateVibeTaskRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: SimpleTaskResponseSchema,
        },
      },
    },
  },
})

/**
 * GET /api/v1/tasks/{taskId}
 * Get task by ID
 */
export const getTaskRoute = createRoute({
  method: 'get',
  path: '/api/v1/tasks/{taskId}',
  tags: ['Tasks'],
  description: 'Retrieve a specific task by its unique identifier',
  request: {
    params: pathParams.taskId,
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: TaskResponseSchema,
        },
      },
    },
  },
})

/**
 * GET /api/v1/tasks
 * List all tasks with pagination
 */
export const listTasksRoute = createRoute({
  method: 'get',
  path: '/api/v1/tasks',
  tags: ['Tasks'],
  description: 'List all tasks with optional pagination and status filtering',
  request: {
    query: TaskListQuerySchema,
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: TaskListResponseSchema,
        },
      },
    },
  },
})

/**
 * DELETE /api/v1/tasks/{taskId}
 * Delete task
 */
export const deleteTaskRoute = createRoute({
  method: 'delete',
  path: '/api/v1/tasks/{taskId}',
  tags: ['Tasks'],
  description:
    'Delete a task and clean up associated resources including workspace, git worktree, and event data',
  request: {
    params: pathParams.taskId,
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: DeleteTaskResponseSchema,
        },
      },
    },
  },
})

/**
 * POST /api/v1/tasks/{taskId}/messages
 * Send message to task
 */
export const sendMessageRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/{taskId}/messages',
  tags: ['Task Messages'],
  description: 'Send a message to an AI agent task to provide additional instructions or feedback',
  request: {
    params: pathParams.taskId,
    body: {
      content: {
        'application/json': {
          schema: SendMessageRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: SendMessageResponseSchema,
        },
      },
    },
  },
})

/**
 * GET /api/v1/tasks/{taskId}/commands
 * Get available commands for task
 */
export const getCommandsRoute = createRoute({
  method: 'get',
  path: '/api/v1/tasks/{taskId}/commands',
  tags: ['Task Commands'],
  description: 'Retrieve list of available AI agent commands for a task based on current state',
  request: {
    params: pathParams.taskId,
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: TaskCommandsResponseSchema,
        },
      },
    },
  },
})

/**
 * POST /api/v1/tasks/{taskId}/commands
 * Send command to task
 */
export const sendCommandRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/{taskId}/commands',
  tags: ['Task Commands'],
  description: 'Execute a predefined command on an AI agent task to perform specific actions',
  request: {
    params: pathParams.taskId,
    body: {
      content: {
        'application/json': {
          schema: TaskCommandSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: SendMessageResponseSchema,
        },
      },
    },
  },
})

/**
 * GET /api/v1/tasks/{taskId}/messages
 * Get task messages with merged tool calls
 */
export const getMessagesRoute = createRoute({
  method: 'get',
  path: '/api/v1/tasks/{taskId}/messages',
  tags: ['Task Messages'],
  description: 'Retrieve all messages for a task with tool calls merged into assistant messages',
  request: {
    params: pathParams.taskId,
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: MessagesResponseSchema,
        },
      },
    },
  },
})

/**
 * POST /api/v1/tasks/{taskId}/start
 * Start task
 */
export const startTaskRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/{taskId}/start',
  tags: ['Task Execution'],
  description: 'Start AI agent execution for a task that was created without auto-start',
  request: {
    params: pathParams.taskId,
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: StartTaskResponseSchema,
        },
      },
    },
  },
})

/**
 * POST /api/v1/tasks/{taskId}/stop
 * Stop agent
 */
export const stopAgentRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/{taskId}/stop',
  tags: ['Task Execution'],
  description: 'Stop the running AI agent for a task while preserving the workspace and task data',
  request: {
    params: pathParams.taskId,
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: StopAgentResponseSchema,
        },
      },
    },
  },
})

/**
 * POST /api/v1/tasks/{taskId}/close
 * Close task
 */
export const closeTaskRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/{taskId}/close',
  tags: ['Tasks'],
  description:
    'Close a task and tear down its workspace, stop dev server, and mark status as closed',
  request: {
    params: pathParams.taskId,
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: CloseTaskResponseSchemaAPI,
        },
      },
    },
  },
})

/**
 * POST /api/v1/tasks/{taskId}/mcp
 * MCP (Model Context Protocol) endpoint for task-scoped tools
 */
export const mcpRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/{taskId}/mcp',
  tags: ['Task MCP'],
  description:
    'Execute Model Context Protocol (MCP) JSON-RPC requests for task-scoped tools and operations',
  request: {
    params: pathParams.taskId,
    body: {
      content: {
        'application/json': {
          schema: McpResponseSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: McpResponseSchema,
        },
      },
    },
  },
})

/**
 * POST /api/v1/tasks/{taskId}/deploy
 * Deploy task to Netlify
 */
export const deployRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/{taskId}/deploy',
  tags: ['Task Deployment'],
  description: 'Deploy task workspace to Netlify for live preview and sharing',
  request: {
    params: pathParams.taskId,
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: DeployResponseSchema,
        },
      },
    },
  },
})

/**
 * POST /api/v1/tasks/{taskId}/user-input-response
 * Submit user response for AskUserQuestion or ExitPlanMode tools
 */
export const userInputResponseRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/{taskId}/user-input-response',
  tags: ['Task Execution'],
  description:
    'Submit user response for interactive tools like AskUserQuestion or ExitPlanMode. The agent pauses waiting for user input and continues after this response is submitted.',
  request: {
    params: pathParams.taskId,
    body: {
      content: {
        'application/json': {
          schema: UserInputResponseRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: z.object({
            success: z.boolean(),
          }),
        },
      },
    },
    400: {
      description: 'Bad Request - missing required fields or tool mismatch',
      content: {
        'application/json': {
          schema: z.object({
            success: z.boolean(),
            error: z.string(),
          }),
        },
      },
    },
    404: {
      description: 'Not Found - no pending user input request',
      content: {
        'application/json': {
          schema: z.object({
            success: z.boolean(),
            error: z.string(),
          }),
        },
      },
    },
    409: {
      description:
        'Conflict - sandbox lost state but session can be resumed. Includes agentSessionId for resuming.',
      content: {
        'application/json': {
          schema: z.object({
            success: z.boolean(),
            error: z.string(),
            canResume: z.boolean(),
            agentSessionId: z.string(),
          }),
        },
      },
    },
    503: {
      description: 'Service Unavailable - sandbox not available',
      content: {
        'application/json': {
          schema: z.object({
            success: z.boolean(),
            error: z.string(),
          }),
        },
      },
    },
  },
})
