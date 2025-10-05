import path from 'node:path'
import { fileURLToPath } from 'node:url'
import type { ServerType } from '@hono/node-server'
import { serve } from '@hono/node-server'
import { config } from 'dotenv'
import { createApp } from './app.js'
import { createLogger } from './utils/logger.js'
import { sseManager } from './utils/sse-manager.js'
import { startWorkerMode } from './worker-app.js'

const __filename = fileURLToPath(import.meta.url)
const __dirname = path.dirname(__filename)

// Load .env from project root (two levels up from apps/backend/)
config({ path: path.resolve(__dirname, '../../.env') })

const logger = createLogger()

// Global references to allow cleanup
let server: ServerType | null = null
let appServices: Awaited<ReturnType<typeof createApp>> | null = null
let isShuttingDown = false

const start = async () => {
  logger.info('Starting application...')

  // Check if running in worker mode
  if (process.env.WORKER_MODE === 'true') {
    if (!process.env.WORKER_API_URL) {
      throw new Error('WORKER_API_URL environment variable not set')
    }
    if (!process.env.WORKER_API_KEY) {
      throw new Error('WORKER_API_KEY environment variable not set')
    }

    if (!process.env.WORKSPACE_DIR) {
      throw new Error('WORKSPACE_DIR environment variable not set')
    }

    const workerConfig = {
      apiUrl: process.env.WORKER_API_URL,
      apiKey: process.env.WORKER_API_KEY,
      workspaceDir: process.env.WORKSPACE_DIR,
    }

    await startWorkerMode(workerConfig)
    return
  }

  try {
    const appWithServices = await createApp()
    appServices = appWithServices
    const { app } = appWithServices

    const port = Number.parseInt(process.env.PORT || '8000', 10)
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

        // 4. Wait a bit for any pending operations
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
      // Better error logging for unhandled rejections
      if (reason instanceof Error) {
        logger.error('Unhandled Rejection:', {
          error: reason.message,
          stack: reason.stack,
          name: reason.name,
          promise: String(promise),
        })
      } else {
        logger.error(`Unhandled Rejection at: ${String(promise)} reason: ${String(reason)}`)
      }

      // Special handling for WritableStream errors during shutdown
      if (
        reason instanceof TypeError &&
        reason.message.includes('WritableStream') &&
        isShuttingDown
      ) {
        logger.debug('Ignoring WritableStream error during shutdown')
        return
      }

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
    server.on('error', (error: Error & { code?: string; syscall?: string; port?: number }) => {
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
