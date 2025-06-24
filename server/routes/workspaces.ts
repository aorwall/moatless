import type { FastifyInstance } from 'fastify'
import { z } from 'zod'
import { WorkspaceError } from '../../shared/constants/errors.js'

// Schema for URL params
const TaskIdParamsSchema = z.object({
  taskId: z.string(),
})

// Schema for file operations
const FilePathParamsSchema = z.object({
  taskId: z.string(),
  '*': z.string(),
})

// Schema for file list query
const FileListQuerySchema = z.object({
  path: z.string().default('.'),
  recursive: z.boolean().default(false),
})

// Schema for file write body
const FileWriteSchema = z.object({
  content: z.string(),
})

export async function registerWorkspaceRoutes(app: FastifyInstance) {
  const { workspace: workspaceService } = app.services

  // Get workspace by task ID
  app.get(
    '/:taskId',
    {
      schema: {
        params: TaskIdParamsSchema,
      },
    },
    async (request, reply) => {
      const { taskId } = request.params
      const workspace = await workspaceService.getWorkspace(taskId)

      if (!workspace) {
        throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId)
      }

      return workspace
    },
  )

  // Get workspace status
  app.get(
    '/:taskId/status',
    {
      schema: {
        params: TaskIdParamsSchema,
      },
    },
    async (request, reply) => {
      const { taskId } = request.params
      const workspace = await workspaceService.getWorkspace(taskId)

      if (!workspace) {
        throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId)
      }

      return {
        status: workspace.status,
        type: workspace.type,
        createdAt: workspace.createdAt,
        error: workspace.error,
      }
    },
  )

  // Cleanup workspace
  app.delete(
    '/:taskId',
    {
      schema: {
        params: TaskIdParamsSchema,
      },
    },
    async (request, reply) => {
      const { taskId } = request.params

      await workspaceService.cleanupWorkspace(taskId)
      return { status: 'cleaned' }
    },
  )

  // Get workspace files (for local workspaces)
  app.get(
    '/:taskId/files',
    {
      schema: {
        params: TaskIdParamsSchema,
        querystring: FileListQuerySchema,
      },
    },
    async (request, reply) => {
      const { taskId } = request.params
      const { path, recursive } = request.query

      // TODO: Implement file listing when workspace service is complete
      return { files: [] }
    },
  )

  // Read workspace file
  app.get(
    '/:taskId/files/*',
    {
      schema: {
        params: FilePathParamsSchema,
      },
    },
    async (request, reply) => {
      const { taskId } = request.params
      const filePath = request.params['*']

      // TODO: Implement file reading when workspace service is complete
      throw new WorkspaceError('File operations not yet implemented', taskId)
    },
  )

  // Write workspace file
  app.put(
    '/:taskId/files/*',
    {
      schema: {
        params: FilePathParamsSchema,
        body: FileWriteSchema,
      },
    },
    async (request, reply) => {
      const { taskId } = request.params
      const filePath = request.params['*']
      const { content } = request.body

      // TODO: Implement file writing when workspace service is complete
      throw new WorkspaceError('File operations not yet implemented', taskId)
    },
  )

  // Get git diff for workspace
  app.get(
    '/:taskId/diff',
    {
      schema: {
        params: TaskIdParamsSchema,
      },
    },
    async (request, reply) => {
      const { taskId } = request.params

      // TODO: Implement git diff when workspace service is complete
      return {
        diff: null,
        files: [],
      }
    },
  )
}
