import {
  CommandExecutionSchema,
  CommandResultSchema,
  FileContentResponseSchema,
  FileContentSchema,
  FileListResponseSchema,
  FileQuerySchema,
  FileSearchResponseSchema,
  FileWriteResponseSchema,
  GitDiffResponseSchema,
  GitStatusSchema,
  TaskFileSearchSchema,
  TaskNotFoundError,
  WorkspaceError,
  WorkspaceStatusResponseSchema,
} from '@moatless/shared'
import type { Hono } from 'hono'
import { HTTPException } from 'hono/http-exception'
import { describeRoute } from 'hono-openapi'
import { resolver } from 'hono-openapi/zod'
import type { AppBindings } from '../app.js'

function handleWorkspaceError(error: unknown): never {
  if (error instanceof TaskNotFoundError) {
    throw new HTTPException(404, { message: 'Task not found' })
  }
  if (error instanceof WorkspaceError) {
    throw new HTTPException(error.statusCode as any, { message: error.message })
  }
  if (error instanceof Error) {
    // Handle specific error messages
    if (error.message.includes('Path not found')) {
      throw new HTTPException(404, { message: error.message })
    }
    if (error.message.includes('Access denied')) {
      throw new HTTPException(403, { message: error.message })
    }
    if (error.message.includes('Cannot list files')) {
      throw new HTTPException(403, { message: error.message })
    }
    if (error.message.includes('File not found')) {
      throw new HTTPException(404, { message: error.message })
    }
    if (error.message.includes('File too large')) {
      throw new HTTPException(413, { message: error.message })
    }
    if (error.message.includes('Path is not a file')) {
      throw new HTTPException(400, { message: error.message })
    }
    if (error.message.includes('Cannot read file')) {
      throw new HTTPException(403, { message: error.message })
    }
    if (error.message.includes('Cannot search files')) {
      throw new HTTPException(403, { message: error.message })
    }
    if (error.message.includes('Cannot access git')) {
      throw new HTTPException(403, { message: error.message })
    }
  }
  throw error
}

function extractFilePathFromWildcard(requestPath: string, prefix: string, taskId: string): string {
  const basePattern = `${prefix}/${taskId}/files/`
  const encodedFilePath = requestPath.replace(basePattern, '')
  const filePath = decodeURIComponent(encodedFilePath)

  if (!filePath) {
    throw new HTTPException(400, { message: 'File path is required' })
  }

  return filePath
}

export async function registerWorkspaceRoutes(app: Hono<AppBindings>, prefix: string) {
  // Get workspace status
  app.get(
    `${prefix}/:taskId/workspace-status`,
    describeRoute({
      description: 'Get workspace status for a task',
      parameters: [
        {
          name: 'taskId',
          in: 'path',
          required: true,
          description: 'Task ID',
          schema: {
            type: 'string',
          },
        },
      ],
      responses: {
        200: {
          description: 'Workspace status information',
          content: {
            'application/json': {
              schema: resolver(WorkspaceStatusResponseSchema),
            },
          },
        },
        400: {
          description: 'Invalid task ID',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = c.req.param('taskId')
      if (!taskId) {
        throw new HTTPException(400, { message: 'Task ID is required' })
      }

      // Task validation now handled in workspace service
      const workspaceStatus = await services.workspace.getWorkspaceStatus(taskId)
      return c.json(workspaceStatus as any)
    },
  )

  // List files in workspace
  app.get(
    `${prefix}/:taskId/files`,
    describeRoute({
      description: 'List files in task workspace',
      parameters: [
        {
          name: 'taskId',
          in: 'path',
          required: true,
          description: 'Task ID',
          schema: {
            type: 'string',
          },
        },
        {
          name: 'path',
          in: 'query',
          required: false,
          description: 'Directory path to list files from',
          schema: {
            type: 'string',
            default: '/',
          },
        },
        {
          name: 'recursive',
          in: 'query',
          required: false,
          description: 'List files recursively',
          schema: {
            type: 'boolean',
            default: false,
          },
        },
      ],
      responses: {
        200: {
          description: 'List of files',
          content: {
            'application/json': {
              schema: resolver(FileListResponseSchema),
            },
          },
        },
        400: {
          description: 'Invalid request',
        },
        403: {
          description: 'Access denied or operation not allowed',
        },
        404: {
          description: 'Task or path not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = c.req.param('taskId')
      if (!taskId) {
        throw new HTTPException(400, { message: 'Task ID is required' })
      }

      const queryParams = c.req.query()
      const validatedQuery = FileQuerySchema.parse(queryParams)
      const { path = '/', recursive = false } = validatedQuery

      try {
        // Task validation and capability check now handled in workspace service
        const files = await services.workspace.listFiles(taskId, path, recursive)
        return c.json({ files, totalCount: files.length })
      } catch (error) {
        handleWorkspaceError(error)
      }
    },
  )

  // Read file content
  app.get(
    `${prefix}/:taskId/files/*`,
    describeRoute({
      description: 'Read file content from task workspace',
      parameters: [
        {
          name: 'taskId',
          in: 'path',
          required: true,
          description: 'Task ID',
          schema: {
            type: 'string',
          },
        },
        {
          name: '*',
          in: 'path',
          required: true,
          description: 'File path',
          schema: {
            type: 'string',
          },
        },
      ],
      responses: {
        200: {
          description: 'File content',
          content: {
            'application/json': {
              schema: resolver(FileContentResponseSchema),
            },
          },
        },
        400: {
          description: 'Invalid request',
        },
        403: {
          description: 'Access denied or operation not allowed',
        },
        404: {
          description: 'Task or file not found',
        },
        413: {
          description: 'File too large',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = c.req.param('taskId')
      if (!taskId) {
        throw new HTTPException(400, { message: 'Task ID is required' })
      }
      const filePath = extractFilePathFromWildcard(c.req.path, prefix, taskId)

      try {
        // Task validation and capability check now handled in workspace service
        const content = await services.workspace.readFile(taskId, filePath)
        return c.json({ content, encoding: 'utf-8' })
      } catch (error) {
        handleWorkspaceError(error)
      }
    },
  )

  // Write file content
  app.put(
    `${prefix}/:taskId/files/*`,
    describeRoute({
      description: 'Write file content to task workspace',
      parameters: [
        {
          name: 'taskId',
          in: 'path',
          required: true,
          description: 'Task ID',
          schema: {
            type: 'string',
          },
        },
        {
          name: '*',
          in: 'path',
          required: true,
          description: 'File path',
          schema: {
            type: 'string',
          },
        },
      ],
      requestBody: {
        description: 'File content',
        content: {
          'application/json': {
            schema: resolver(FileContentSchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'File written successfully',
          content: {
            'application/json': {
              schema: resolver(FileWriteResponseSchema),
            },
          },
        },
        400: {
          description: 'Invalid request',
        },
        403: {
          description: 'Access denied or operation not allowed',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = c.req.param('taskId')
      if (!taskId) {
        throw new HTTPException(400, { message: 'Task ID is required' })
      }
      const filePath = extractFilePathFromWildcard(c.req.path, prefix, taskId)

      const body = await c.req.json()
      const validatedBody = FileContentSchema.parse(body)
      const { content } = validatedBody

      // Task validation and capability check now handled in workspace service
      await services.workspace.writeFile(taskId, filePath, content)
      return c.json({ success: true })
    },
  )

  // Search files by filename/path
  app.post(
    `${prefix}/:taskId/search-files`,
    describeRoute({
      description: 'Search files by filename/path in task workspace',
      parameters: [
        {
          name: 'taskId',
          in: 'path',
          required: true,
          description: 'Task ID',
          schema: {
            type: 'string',
          },
        },
      ],
      requestBody: {
        description: 'Search parameters',
        content: {
          'application/json': {
            schema: resolver(TaskFileSearchSchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'Search results',
          content: {
            'application/json': {
              schema: resolver(FileSearchResponseSchema),
            },
          },
        },
        400: {
          description: 'Invalid request',
        },
        403: {
          description: 'Operation not allowed',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = c.req.param('taskId')
      if (!taskId) {
        throw new HTTPException(400, { message: 'Task ID is required' })
      }

      const body = await c.req.json()
      const validatedBody = TaskFileSearchSchema.parse(body)
      const { query, limit } = validatedBody

      try {
        // Task validation and capability check now handled in workspace service
        const result = await services.workspace.searchFiles(taskId, {
          query,
          limit,
        })
        return c.json(result)
      } catch (error) {
        handleWorkspaceError(error)
      }
    },
  )

  // Get git diff for workspace
  app.get(
    `${prefix}/:taskId/git-diff`,
    describeRoute({
      description: 'Get git diff for task workspace',
      parameters: [
        {
          name: 'taskId',
          in: 'path',
          required: true,
          description: 'Task ID',
          schema: {
            type: 'string',
          },
        },
      ],
      responses: {
        200: {
          description: 'Git diff',
          content: {
            'application/json': {
              schema: resolver(GitDiffResponseSchema),
            },
          },
        },
        400: {
          description: 'Invalid task ID',
        },
        403: {
          description: 'Cannot access git',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = c.req.param('taskId')
      if (!taskId) {
        throw new HTTPException(400, { message: 'Task ID is required' })
      }

      try {
        // Task validation and capability check now handled in workspace service
        const diffResult = await services.workspace.getGitDiff(taskId)
        return c.json({
          diff: diffResult.diffString,
          files: diffResult.files.map((f) => f.filename),
          hasChanges: diffResult.files.length > 0,
        })
      } catch (error) {
        handleWorkspaceError(error)
      }
    },
  )

  // Get git sync status with origin for workspace
  app.get(
    `${prefix}/:taskId/git-sync-status`,
    describeRoute({
      description: 'Get git sync status with origin for task workspace',
      parameters: [
        {
          name: 'taskId',
          in: 'path',
          required: true,
          description: 'Task ID',
          schema: {
            type: 'string',
          },
        },
      ],
      responses: {
        200: {
          description: 'Git sync status',
          content: {
            'application/json': {
              schema: resolver(GitStatusSchema),
            },
          },
        },
        400: {
          description: 'Invalid task ID',
        },
        403: {
          description: 'Cannot access git',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = c.req.param('taskId')
      if (!taskId) {
        throw new HTTPException(400, { message: 'Task ID is required' })
      }

      try {
        // Get workspace status which now includes unified git status
        const workspaceStatus = await services.workspace.getWorkspaceStatus(taskId)

        if ((workspaceStatus as any).gitStatus) {
          return c.json((workspaceStatus as any).gitStatus)
        } else {
          // If git status is not available, return a default response
          return c.json({
            currentBranch: 'unknown',
            baseBranch: 'main',
            hasUncommittedChanges: false,
            hasConflicts: false,
            status: 'error',
            commitsAhead: 0,
            commitsBehind: 0,
            originExists: false,
          })
        }
      } catch (error) {
        handleWorkspaceError(error)
      }
    },
  )

  // Sync workspace with main branch
  app.post(
    `${prefix}/:taskId/sync-with-main`,
    describeRoute({
      description: 'Safely sync workspace with main branch if no conflicts exist',
      parameters: [
        {
          name: 'taskId',
          in: 'path',
          required: true,
          description: 'Task ID',
          schema: {
            type: 'string',
          },
        },
        {
          name: 'baseBranch',
          in: 'query',
          required: false,
          description: 'Base branch to sync with (defaults to main)',
          schema: {
            type: 'string',
            default: 'main',
          },
        },
      ],
      responses: {
        200: {
          description: 'Sync operation result',
          content: {
            'application/json': {
              schema: {
                type: 'object',
                properties: {
                  success: {
                    type: 'boolean',
                    description: 'Whether the sync was successful',
                  },
                  conflicts: {
                    type: 'boolean',
                    description: 'Whether conflicts were detected',
                  },
                  message: {
                    type: 'string',
                    description: 'Description of the sync result',
                  },
                  commitsBehind: {
                    type: 'number',
                    description: 'Number of commits behind main',
                  },
                  commitsAhead: {
                    type: 'number',
                    description: 'Number of commits ahead of main',
                  },
                },
                required: ['success', 'conflicts', 'message'],
              },
            },
          },
        },
        400: {
          description: 'Invalid request',
        },
        403: {
          description: 'Cannot access git or workspace not ready',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = c.req.param('taskId')
      if (!taskId) {
        throw new HTTPException(400, { message: 'Task ID is required' })
      }

      const baseBranch = c.req.query('baseBranch')

      try {
        // Task validation and capability check now handled in workspace service
        const result = await services.workspace.syncWorkspaceWithMain(taskId, baseBranch)
        return c.json(result)
      } catch (error) {
        handleWorkspaceError(error)
      }
    },
  )

  // Execute command in workspace
  app.post(
    `${prefix}/:taskId/execute`,
    describeRoute({
      description: 'Execute a command in task workspace',
      parameters: [
        {
          name: 'taskId',
          in: 'path',
          required: true,
          description: 'Task ID',
          schema: {
            type: 'string',
          },
        },
      ],
      requestBody: {
        description: 'Command execution parameters',
        content: {
          'application/json': {
            schema: resolver(CommandExecutionSchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'Command execution result',
          content: {
            'application/json': {
              schema: resolver(CommandResultSchema),
            },
          },
        },
        400: {
          description: 'Invalid request',
        },
        403: {
          description: 'Operation not allowed',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = c.req.param('taskId')
      if (!taskId) {
        throw new HTTPException(400, { message: 'Task ID is required' })
      }

      const body = await c.req.json()
      const validatedBody = CommandExecutionSchema.parse(body)
      const { command, timeout } = validatedBody

      // Task validation and capability check now handled in workspace service
      const result = await services.workspace.runCommand(taskId, command, timeout)
      return c.json(result)
    },
  )
}
