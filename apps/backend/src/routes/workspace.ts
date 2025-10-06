import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  executeCommandRoute,
  getFileTreeWithChangesRoute,
  getWorkspaceGitDiffRoute,
  getWorkspaceStatusRoute,
  listWorkspaceFilesRoute,
  readWorkspaceFileRoute,
  searchWorkspaceFilesRoute,
  setupWorkspaceRoute,
  writeWorkspaceFileRoute,
} from '@moatless/api-contract/workspace'
import type { AppBindings } from '../app.js'
import { handleServiceError } from '../utils/error-handler.js'

export async function registerWorkspaceRoutes(app: OpenAPIHono<AppBindings>) {
  // GET /api/v1/tasks/:taskId/workspace-status
  app.openapi(getWorkspaceStatusRoute, async (c) => {
    const services = c.get('services')
    const { taskId } = c.req.valid('param')

    try {
      const workspaceStatus = await services.workspace.getWorkspaceStatus(taskId)
      return c.json(workspaceStatus)
    } catch (error) {
      handleServiceError(error, 'Get workspace status')
    }
  })

  // GET /api/v1/tasks/:taskId/files
  app.openapi(listWorkspaceFilesRoute, async (c) => {
    const services = c.get('services')
    const { taskId } = c.req.valid('param')
    const { path = '/', recursive = false } = c.req.valid('query')

    try {
      const workspace = await services.workspace.getOrCreateWorkspace(taskId)
      const fileList = await workspace.listFiles(path, recursive)

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
    const services = c.get('services')
    const { taskId, filePath } = c.req.valid('param')

    try {
      const workspace = await services.workspace.getOrCreateWorkspace(taskId)
      const fileData = await workspace.readFileWithChanges(filePath)
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
    const services = c.get('services')
    const { taskId, filePath } = c.req.valid('param')
    const { content } = c.req.valid('json')

    try {
      const workspace = await services.workspace.getOrCreateWorkspace(taskId)
      await workspace.writeFile(filePath, content)
      return c.json({ success: true })
    } catch (error) {
      handleServiceError(error, 'Write file')
    }
  })

  // POST /api/v1/tasks/:taskId/search-files
  app.openapi(searchWorkspaceFilesRoute, async (c) => {
    const services = c.get('services')
    const { taskId } = c.req.valid('param')
    const { query, limit } = c.req.valid('json')

    try {
      const workspace = await services.workspace.getOrCreateWorkspace(taskId)
      const result = await workspace.searchFiles({
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
    const services = c.get('services')
    const { taskId } = c.req.valid('param')

    try {
      const workspace = await services.workspace.getOrCreateWorkspace(taskId)
      const result = await workspace.getGitDiff()
      return c.json(result)
    } catch (error) {
      handleServiceError(error, 'Get git diff')
    }
  })

  // GET /api/v1/tasks/:taskId/file-tree-with-changes
  app.openapi(getFileTreeWithChangesRoute, async (c) => {
    const services = c.get('services')
    const { taskId } = c.req.valid('param')
    const { path: rootPath = '.' } = c.req.valid('query')

    try {
      const workspace = await services.workspace.getOrCreateWorkspace(taskId)
      const result = await workspace.getFileTreeWithChanges(rootPath)
      return c.json(result)
    } catch (error) {
      handleServiceError(error, 'Get file tree with changes')
    }
  })

  // POST /api/v1/tasks/:taskId/execute
  app.openapi(executeCommandRoute, async (c) => {
    const services = c.get('services')
    const { taskId } = c.req.valid('param')
    const { command, timeout } = c.req.valid('json')

    try {
      const workspace = await services.workspace.getOrCreateWorkspace(taskId)
      const result = await workspace.runCommand(command, timeout)
      return c.json(result)
    } catch (error) {
      handleServiceError(error, 'Execute command')
    }
  })

  // POST /api/v1/tasks/:taskId/setup
  app.openapi(setupWorkspaceRoute, async (c) => {
    const services = c.get('services')
    const { taskId } = c.req.valid('param')

    try {
      await services.workspace.setupWorkspace(taskId)

      return c.json({
        success: true,
        message: 'Workspace setup completed successfully',
        status: 'ready' as const,
      })
    } catch (error) {
      handleServiceError(error, 'Setup workspace')
    }
  })
}
