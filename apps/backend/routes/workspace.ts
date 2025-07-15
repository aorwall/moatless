import {
  CommandExecutionSchema,
  CommandResultSchema,
  FileContentResponseSchema,
  FileContentSchema,
  FileListResponseSchema,
  FileQuerySchema,
  FileSearchResponseSchema,
  FileTreeWithChangesResponseSchema,
  FileWriteResponseSchema,
  GitDiffResponseSchema,
  GitStatusSchema,
  SyncWorkspaceResponseSchema,
  TaskFileSearchSchema,
  WorkspaceStatusResponseSchema,
} from '@moatless/types'
import type { Hono } from 'hono'
import { describeRoute } from 'hono-openapi'
import { resolver } from 'hono-openapi/zod'
import type { AppBindings } from '../app.js'
import { getTaskId, handleServiceError } from '../utils/error-handler.js'
import {
  CommonParameters,
  extractFilePathFromWildcard,
  validateQueryParams,
  validateRequestBody,
} from '../utils/route-helpers.js'

export async function registerWorkspaceRoutes(app: Hono<AppBindings>, prefix: string) {
  // Get workspace status
  app.get(
    `${prefix}/:taskId/workspace-status`,
    describeRoute({
      description: 'Get workspace status for a task',
      parameters: [CommonParameters.taskId],
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
      const taskId = getTaskId(c)

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
      const taskId = getTaskId(c)

      const validatedQuery = validateQueryParams(c, FileQuerySchema)
      const { path = '/', recursive = false } = validatedQuery

      try {
        // Task validation and capability check now handled in workspace service
        const files = await services.workspace.listFiles(taskId, path, recursive)
        return c.json({ files, totalCount: files.length })
      } catch (error) {
        handleServiceError(error, 'Get workspace status')
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
      const taskId = getTaskId(c)
      const filePath = extractFilePathFromWildcard(c.req.path, prefix, taskId)

      try {
        // Task validation and capability check now handled in workspace service
        const content = await services.workspace.readFile(taskId, filePath)
        return c.json({ content, encoding: 'utf-8' })
      } catch (error) {
        handleServiceError(error, 'Get workspace status')
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
      const taskId = getTaskId(c)
      const filePath = extractFilePathFromWildcard(c.req.path, prefix, taskId)

      const validatedBody = await validateRequestBody(c, FileContentSchema)
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
      parameters: [CommonParameters.taskId],
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
      const taskId = getTaskId(c)

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
        handleServiceError(error, 'Get workspace status')
      }
    },
  )

  // Get git diff for workspace
  app.get(
    `${prefix}/:taskId/git-diff`,
    describeRoute({
      description: 'Get git diff for task workspace showing branch changes',
      parameters: [CommonParameters.taskId],
      responses: {
        200: {
          description: 'Git diff showing changes on the branch',
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
      const taskId = getTaskId(c)

      try {
        // Task validation and capability check now handled in workspace service
        const diffResult = await services.workspace.getGitDiff(taskId)
        return c.json({
          diff: diffResult.diffString,
          files: diffResult.files.map((f) => f.filename),
          hasChanges: diffResult.files.length > 0,
          totalChanges: diffResult.totalChanges,
          totalAdditions: diffResult.totalAdditions,
          totalDeletions: diffResult.totalDeletions,
          compareBase: diffResult.compareBase,
        })
      } catch (error) {
        handleServiceError(error, 'Get workspace status')
      }
    },
  )

  // Get git sync status with origin for workspace
  app.get(
    `${prefix}/:taskId/git-sync-status`,
    describeRoute({
      description: 'Get git sync status with origin for task workspace',
      parameters: [CommonParameters.taskId],
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
      const taskId = getTaskId(c)

      try {
        // Get workspace status which now includes unified git status
        const workspaceStatus = await services.workspace.getWorkspaceStatus(taskId)

        if ((workspaceStatus as any).gitStatus) {
          return c.json((workspaceStatus as any).gitStatus)
        }
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
      } catch (error) {
        handleServiceError(error, 'Get workspace status')
      }
    },
  )

  // Get file tree with git changes
  app.get(
    `${prefix}/:taskId/file-tree-with-changes`,
    describeRoute({
      description: 'Get file tree with git change information',
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
          description: 'Root directory path (defaults to workspace root)',
          schema: {
            type: 'string',
            default: '.',
          },
        },
      ],
      responses: {
        200: {
          description: 'File tree with git change information',
          content: {
            'application/json': {
              schema: resolver(FileTreeWithChangesResponseSchema),
            },
          },
        },
        400: {
          description: 'Invalid task ID',
        },
        403: {
          description: 'Cannot access files: workspace not ready',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = getTaskId(c)
      const rootPath = c.req.query('path') || '.'

      try {
        const result = await services.workspace.getFileTreeWithChanges(taskId, rootPath)
        return c.json(result)
      } catch (error) {
        handleServiceError(error, 'Get file tree with changes')
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
              schema: resolver(SyncWorkspaceResponseSchema),
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
      const taskId = getTaskId(c)

      const baseBranch = c.req.query('baseBranch')

      try {
        // Task validation and capability check now handled in workspace service
        const result = await services.workspace.syncWorkspaceWithMain(taskId, baseBranch)
        return c.json(result)
      } catch (error) {
        handleServiceError(error, 'Get workspace status')
      }
    },
  )

  // Execute command in workspace
  app.post(
    `${prefix}/:taskId/execute`,
    describeRoute({
      description: 'Execute a command in task workspace',
      parameters: [CommonParameters.taskId],
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
      const taskId = getTaskId(c)

      const body = await c.req.json()
      const validatedBody = CommandExecutionSchema.parse(body)
      const { command, timeout } = validatedBody

      // Task validation and capability check now handled in workspace service
      const result = await services.workspace.runCommand(taskId, command, timeout)
      return c.json(result)
    },
  )
}
