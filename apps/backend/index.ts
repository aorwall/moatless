import type { ServerType } from '@hono/node-server'
import { serve } from '@hono/node-server'
import { config } from 'dotenv'
import { createApp } from './app.js'
import { createLogger } from './utils/logger.js'
import { sseManager } from './utils/sse-manager.js'
import { startWorkerMode } from './worker-mode.js'

config() // Load environment variables

const logger = createLogger()

// Global references to allow cleanup
let server: ServerType | null = null
let appServices: Awaited<ReturnType<typeof createApp>> | null = null

const start = async () => {
  // Check if running in worker mode
  if (process.env.WORKER_MODE === 'true') {
    const workerConfig = {
      apiUrl: process.env.WORKER_API_URL || 'http://localhost:8000',
      apiKey: process.env.WORKER_API_KEY,
      workspaceDir: process.env.WORKSPACE_DIR || '/workspace',
    }

    await startWorkerMode(workerConfig)
    return
  }

  try {
    const appWithServices = await createApp()
    appServices = appWithServices
    const { app } = appWithServices

    const port = parseInt(process.env.PORT || '8000', 10)
    const host = process.env.HOST || '0.0.0.0'

    // Setup graceful shutdown handlers
    const gracefulShutdown = async (signal: string) => {
      logger.info(`Received ${signal}, starting graceful shutdown...`)

      try {
        // 1. Stop accepting new connections
        if (server) {
          logger.info('Stopping HTTP server...')
          server.close()
        }

        // 2. Close SSE connections
        logger.info('Closing SSE connections...')
        await sseManager.gracefulShutdown()

        // 3. Shutdown preview environments
        if (appServices?.services.previewEnvironment) {
          logger.info('Shutting down preview environments...')
          await appServices.services.previewEnvironment.gracefulShutdown()
        }

        // 4. Wait a bit for any pending operations
        logger.info('Waiting for pending operations...')
        await new Promise((resolve) => setTimeout(resolve, 1000))

        logger.info('Graceful shutdown completed')
        process.exit(0)
      } catch (error) {
        logger.error('Error during graceful shutdown:', error)
        process.exit(1)
      }
    }

    // Handle shutdown signals
    process.on('SIGTERM', () => gracefulShutdown('SIGTERM'))
    process.on('SIGINT', () => gracefulShutdown('SIGINT'))

    // Handle unhandled promise rejections
    process.on('unhandledRejection', (reason, promise) => {
      logger.error(`Unhandled Rejection at: ${String(promise)} reason: ${String(reason)}`)
      // Don't exit the process - allow the server to continue running
    })

    // Handle uncaught exceptions
    process.on('uncaughtException', (error) => {
      logger.error('Uncaught Exception:', error)
      // For uncaught exceptions, we should exit as the process state may be corrupted
      gracefulShutdown('UNCAUGHT_EXCEPTION')
    })

    server = serve({
      fetch: app.fetch,
      port,
      hostname: host,
    })

    logger.info(`Server listening on http://${host}:${port}`)
    console.log(`Server listening on http://${host}:${port}`)
  } catch (err) {
    logger.error(`Failed to start server: ${String(err)}`)
    process.exit(1)
  }
}

start()
