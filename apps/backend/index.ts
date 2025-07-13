import type { ServerType } from '@hono/node-server'
import { serve } from '@hono/node-server'
import { config } from 'dotenv'
import { createApp } from './app.js'
import { createLogger } from './utils/logger.js'
import { sseManager } from './utils/sse-manager.js'
import { startWorkerMode } from './worker-app.js'

config() // Load environment variables

const logger = createLogger()

// Global references to allow cleanup
let server: ServerType | null = null
let appServices: Awaited<ReturnType<typeof createApp>> | null = null
let isShuttingDown = false

const start = async () => {
  console.log('Starting application...')
  logger.info('Starting application...')

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
      // Prevent multiple shutdown attempts
      if (isShuttingDown) {
        logger.warn(`Already shutting down, ignoring ${signal}`)
        return
      }
      isShuttingDown = true

      logger.info(`Received ${signal}, starting graceful shutdown...`)

      // Add a timeout to force exit if shutdown hangs
      const shutdownTimeout = setTimeout(() => {
        logger.error('Graceful shutdown timeout, forcing exit')
        process.exit(1)
      }, 30000) // 30 second timeout

      try {
        // 1. Stop accepting new connections
        if (server) {
          logger.info('Stopping HTTP server...')
          server.close()
        }

        // 2. Close SSE connections
        logger.info('Closing SSE connections...')
        await sseManager.gracefulShutdown()

        // 3. Shutdown workspaces (including Docker containers)
        if (appServices?.services.workspace) {
          logger.info('Shutting down workspaces...')
          await appServices.services.workspace.gracefulShutdown()
        }

        // 4. Shutdown preview environments
        if (appServices?.services.previewEnvironment) {
          logger.info('Shutting down preview environments...')
          await appServices.services.previewEnvironment.gracefulShutdown()
        }

        // 5. Wait a bit for any pending operations
        logger.info('Waiting for pending operations...')
        await new Promise((resolve) => setTimeout(resolve, 1000))

        clearTimeout(shutdownTimeout)
        logger.info('Graceful shutdown completed')
        process.exit(0)
      } catch (error) {
        logger.error('Error during graceful shutdown:', error)
        clearTimeout(shutdownTimeout)
        process.exit(1)
      }
    }

    // Handle shutdown signals
    process.on('SIGTERM', () => {
      logger.info('Received SIGTERM signal')
      gracefulShutdown('SIGTERM')
    })
    process.on('SIGINT', () => {
      logger.info('Received SIGINT signal')
      gracefulShutdown('SIGINT')
    })

    // Handle unhandled promise rejections
    process.on('unhandledRejection', (reason, promise) => {
      logger.error(`Unhandled Rejection at: ${String(promise)} reason: ${String(reason)}`)
      // Don't exit the process - allow the server to continue running
    })

    // Handle uncaught exceptions
    process.on('uncaughtException', (error) => {
      logger.error('Uncaught Exception:', error)
      console.error('Uncaught Exception:', error)
      // For uncaught exceptions, we should exit as the process state may be corrupted
      gracefulShutdown('UNCAUGHT_EXCEPTION')
    })

    // Log all process exit events
    process.on('exit', (code) => {
      logger.info(`Process exiting with code: ${code}`)
    })

    server = serve({
      fetch: app.fetch,
      port,
      hostname: host,
    })

    // Add error handling for server
    server.on('error', (error: any) => {
      logger.error('Server error:', error)
      if (error.code === 'EADDRINUSE') {
        logger.error(`Port ${port} is already in use`)
        console.error(
          `Error: Port ${port} is already in use. Please stop the other process or use a different port.`,
        )
        process.exit(1)
      } else {
        logger.error('Failed to start server:', error)
        process.exit(1)
      }
    })

    logger.info(`Server listening on http://${host}:${port}`)
    console.log(`Server listening on http://${host}:${port}`)
  } catch (err) {
    logger.error(`Failed to start server: ${String(err)}`)
    process.exit(1)
  }
}

start()
