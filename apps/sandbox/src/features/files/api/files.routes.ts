import type { Hono } from 'hono'
import type { AppBindings } from '../../../app'
import { logger } from '../../../shared/logger'
import { FileService } from '../services/file.service'

/**
 * Register file operation routes
 * These routes match KubernetesEnvironmentDriver expectations
 */
export function registerFileRoutes(app: Hono<AppBindings>) {
  // POST /read-file - Read a file from workspace
  app.post('/read-file', async (c) => {
    const { config } = c.var
    const fileService = new FileService(config)

    try {
      const body = await c.req.json()
      const { filePath } = body as { filePath: string }

      if (!filePath) {
        return c.json({ error: 'filePath is required' }, 400)
      }

      const content = await fileService.readFile(filePath)
      return c.json({ content })
    } catch (error) {
      logger.error({ err: error as Error, taskId: config.taskId }, 'Failed to read file')
      return c.json({ error: error instanceof Error ? error.message : String(error) }, 500)
    }
  })

  // POST /write-file - Write a file to workspace
  app.post('/write-file', async (c) => {
    const { config } = c.var
    const fileService = new FileService(config)

    try {
      const body = await c.req.json()
      const { filePath, content } = body as { filePath: string; content: string }

      if (!filePath) {
        return c.json({ success: false, error: 'filePath is required' }, 400)
      }

      if (content === undefined) {
        return c.json({ success: false, error: 'content is required' }, 400)
      }

      await fileService.writeFile(filePath, content)
      return c.json({ success: true })
    } catch (error) {
      logger.error({ err: error as Error, taskId: config.taskId }, 'Failed to write file')
      return c.json(
        { success: false, error: error instanceof Error ? error.message : String(error) },
        500,
      )
    }
  })

  // POST /list-files - List files in workspace
  app.post('/list-files', async (c) => {
    const { config } = c.var
    const fileService = new FileService(config)

    try {
      const body = await c.req.json()
      const { relativePath, recursive } = body as { relativePath?: string; recursive?: boolean }

      const files = await fileService.listFiles(relativePath, recursive)
      return c.json({ files })
    } catch (error) {
      logger.error({ err: error as Error, taskId: config.taskId }, 'Failed to list files')
      return c.json(
        { files: [], error: error instanceof Error ? error.message : String(error) },
        500,
      )
    }
  })

  // POST /search-files - Search for files in workspace
  app.post('/search-files', async (c) => {
    const { config } = c.var
    const fileService = new FileService(config)

    try {
      const body = await c.req.json()
      const result = await fileService.searchFiles(body)
      return c.json(result)
    } catch (error) {
      logger.error({ err: error as Error, taskId: config.taskId }, 'Failed to search files')
      return c.json(
        {
          files: [],
          totalMatches: 0,
          error: error instanceof Error ? error.message : String(error),
        },
        500,
      )
    }
  })

  // POST /file-tree-with-changes - Get file tree with git info (driver format)
  app.post('/file-tree-with-changes', async (c) => {
    const { config } = c.var
    const fileService = new FileService(config)

    try {
      const body = await c.req.json()
      const { rootPath = '.' } = body as { rootPath?: string }
      const result = await fileService.getFileTreeWithChanges(rootPath)
      return c.json(result)
    } catch (error) {
      logger.error({ err: error as Error, taskId: config.taskId }, 'Failed to get file tree')
      return c.json({ error: error instanceof Error ? error.message : String(error) }, 500)
    }
  })

  // GET /file-tree-with-changes - Get file tree with git info (convenience)
  app.get('/file-tree-with-changes', async (c) => {
    const { config } = c.var
    const fileService = new FileService(config)

    try {
      const rootPath = c.req.query('path') || '.'
      const result = await fileService.getFileTreeWithChanges(rootPath)
      return c.json(result)
    } catch (error) {
      logger.error({ err: error as Error, taskId: config.taskId }, 'Failed to get file tree')
      return c.json({ error: error instanceof Error ? error.message : String(error) }, 500)
    }
  })

  // POST /read-file-with-changes - Read file with git change info
  app.post('/read-file-with-changes', async (c) => {
    const { config } = c.var
    const fileService = new FileService(config)

    try {
      const body = await c.req.json()
      const { filePath } = body as { filePath: string }

      if (!filePath) {
        return c.json({ error: 'filePath is required' }, 400)
      }

      const result = await fileService.readFileWithChanges(filePath)
      return c.json(result)
    } catch (error) {
      logger.error(
        { err: error as Error, taskId: config.taskId },
        'Failed to read file with changes',
      )
      return c.json({ error: error instanceof Error ? error.message : String(error) }, 500)
    }
  })
}
