import type { Hono } from 'hono'
import type { AppBindings } from '../../../app'
import { logger } from '../../../shared/logger'
import { FileService } from '../services/file.service'

/**
 * Register file operation routes
 * These routes match KubernetesEnvironmentDriver expectations
 */
export function registerFileRoutes(app: Hono<AppBindings>) {
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
}
