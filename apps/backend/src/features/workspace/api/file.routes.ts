import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  getFileTreeWithChangesRoute,
  getWorkspaceGitDiffRoute,
  listWorkspaceFilesRoute,
  readWorkspaceFileRoute,
  searchWorkspaceFilesRoute,
  writeWorkspaceFileRoute,
} from '@moatless/api-contract/workspace'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { handleServiceError } from '../../../utils/error-handler'
import { FileGateway } from '../services/file-gateway.service'

/**
 * Register file operation routes for workspace feature
 *
 * These routes use the new FileGateway architecture with taskId-only API:
 * - FileGateway handles workspace path resolution via WorkspaceRuntimeManager
 * - File operations are delegated to FileOperations service (local mode)
 * - Routes no longer need to manage workspace provisioning or path resolution
 *
 * Note: executeCommand route still uses legacy WorkspaceService (not a file operation)
 */
export function registerFileRoutes(app: OpenAPIHono<AppBindings>) {
  // GET /api/v1/tasks/:taskId/files
  app.openapi(listWorkspaceFilesRoute, async (c) => {
    const fileGateway = appContainer.get(FileGateway)
    const { taskId } = c.req.valid('param')
    const { path = '/', recursive = false } = c.req.valid('query')

    try {
      const fileList = await fileGateway.listFiles(taskId, path, recursive)

      const files = fileList.map((filePath) => ({
        name: filePath.split('/').pop() || filePath,
        path: filePath,
        type: 'file' as const,
      }))

      return c.json({ files, totalCount: files.length })
    } catch (error) {
      handleServiceError(error, 'List files')
    }
  })

  // GET /api/v1/tasks/:taskId/files/{filePath}
  app.openapi(readWorkspaceFileRoute, async (c) => {
    const fileGateway = appContainer.get(FileGateway)
    const { taskId, filePath } = c.req.valid('param')

    try {
      const fileData = await fileGateway.readFileWithChanges(taskId, filePath)
      return c.json({
        content: fileData.content,
        encoding: 'utf-8',
        oldContent: fileData.oldContent,
        additions: fileData.additions,
        deletions: fileData.deletions,
        isModified: fileData.isModified,
      })
    } catch (error) {
      handleServiceError(error, 'Read file')
    }
  })

  // PUT /api/v1/tasks/:taskId/files/{filePath}
  app.openapi(writeWorkspaceFileRoute, async (c) => {
    const fileGateway = appContainer.get(FileGateway)
    const { taskId, filePath } = c.req.valid('param')
    const { content } = c.req.valid('json')

    try {
      await fileGateway.writeFile(taskId, filePath, content)
      return c.json({ success: true })
    } catch (error) {
      handleServiceError(error, 'Write file')
    }
  })

  // POST /api/v1/tasks/:taskId/search-files
  app.openapi(searchWorkspaceFilesRoute, async (c) => {
    const fileGateway = appContainer.get(FileGateway)
    const { taskId } = c.req.valid('param')
    const { query, limit } = c.req.valid('json')

    try {
      const result = await fileGateway.searchFiles(taskId, {
        query,
        ...(limit !== undefined && { limit }),
      })
      return c.json(result)
    } catch (error) {
      handleServiceError(error, 'Search files')
    }
  })

  // GET /api/v1/tasks/:taskId/git-diff
  app.openapi(getWorkspaceGitDiffRoute, async (c) => {
    const fileGateway = appContainer.get(FileGateway)
    const { taskId } = c.req.valid('param')

    try {
      const result = await fileGateway.getGitDiff(taskId)
      return c.json(result)
    } catch (error) {
      handleServiceError(error, 'Get git diff')
    }
  })

  // GET /api/v1/tasks/:taskId/file-tree-with-changes
  app.openapi(getFileTreeWithChangesRoute, async (c) => {
    const fileGateway = appContainer.get(FileGateway)
    const { taskId } = c.req.valid('param')
    const { path: rootPath = '.' } = c.req.valid('query')

    try {
      const result = await fileGateway.getFileTreeWithChanges(taskId, rootPath)
      return c.json(result)
    } catch (error) {
      handleServiceError(error, 'Get file tree with changes')
    }
  })
}
