import type { ServerType } from '@hono/node-server'
import { serve } from '@hono/node-server'
import { createApp } from './app'
import { sseManager } from './features/events/index'
import { memoryMonitor } from './services/memory-monitor.service'
import { initializeTelemetry, shutdownTelemetry } from './telemetry'
import { logger } from './utils/logger'

// Bun automatically loads .env files, so no need for dotenv package
// When running with bun --env-file=.env.test, that file will be used instead

// Global references to allow cleanup
let server: ServerType | null = null
let isShuttingDown = false

const start = async () => {
  logger.info('Starting application...')

  // Initialize telemetry before anything else
  await initializeTelemetry()

  try {
    const { app } = await createApp()

    const port = Number.parseInt(process.env['PORT'] || '8000', 10)
    const host = process.env['HOST'] || '0.0.0.0'

    // Setup graceful shutdown handlers
    const gracefulShutdown = async (signal: string) => {
      // Prevent multiple shutdown attempts
      if (isShuttingDown) {
        logger.warn({ signal }, 'Already shutting down')
        return
      }
      isShuttingDown = true

      logger.info({ signal }, 'Received signal, starting graceful shutdown')

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

        // 2. Stop memory monitoring
        memoryMonitor.stop()

        // 2.5. Stop event services (NOTIFY listener, poller, message subscription)
        const { NotifyListenerService } = await import(
          './features/events/services/notify-listener.service.js'
        )
        const { EventPollerService } = await import(
          './features/events/services/event-poller.service.js'
        )
        const { MessageService } = await import('./features/agents/services/message.service.js')
        const { appContainer: container } = await import('./container/container.js')
        if (container) {
          // Stop NOTIFY/LISTEN first (primary event delivery)
          const notifyListener = container.get(NotifyListenerService)
          await notifyListener.stop()

          // Stop event poller (fallback)
          const eventPoller = container.get(EventPollerService)
          eventPoller.stop()

          const messageService = container.get(MessageService)
          messageService.stopEventSubscription()
        }

        // 3. Close SSE connections
        logger.info('Closing SSE connections...')
        await sseManager.gracefulShutdown()

        // 4. Mark active iterations as failed before shutdown
        const { appContainer } = await import('./container/container.js')
        const { TaskIterationService } = await import(
          './features/tasks/services/task-iteration.service.js'
        )

        if (appContainer) {
          const taskIterationService = appContainer.get(TaskIterationService)
          await taskIterationService.cleanupActiveIterationsOnShutdown()
        }

        // 5. Close database connection to ensure WAL data is checkpointed
        const { DatabaseSymbol } = await import('./infrastructure/database/database.js')
        if (appContainer) {
          const database = appContainer.get(DatabaseSymbol)
          logger.info('Closing database connection...')
          await database.close()
        }

        // 6. Shutdown telemetry (flush remaining data)
        await shutdownTelemetry()

        // 7. Wait a bit for any pending operations
        logger.info('Waiting for pending operations...')
        await new Promise((resolve) => setTimeout(resolve, 1000))

        clearTimeout(shutdownTimeout)
        logger.info('Graceful shutdown completed')
        process.exit(0)
      } catch (error) {
        logger.error({ err: error }, 'Error during graceful shutdown')
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
        logger.error(
          {
            error: reason.message,
            stack: reason.stack,
            name: reason.name,
            promise: String(promise),
          },
          'Unhandled Rejection',
        )
      } else {
        logger.error({ promise: String(promise), reason: String(reason) }, 'Unhandled Rejection')
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
      logger.error({ err: error }, 'Uncaught Exception')
      // For uncaught exceptions, we should exit as the process state may be corrupted
      gracefulShutdown('UNCAUGHT_EXCEPTION')
    })

    // Log all process exit events
    process.on('exit', (code) => {
      logger.info({ code }, 'Process exiting')
    })

    server = serve({
      fetch: app.fetch,
      port,
      hostname: host,
    })

    // Add error handling for server
    server.on('error', (error: Error & { code?: string; syscall?: string; port?: number }) => {
      logger.error({ err: error }, 'Server error')
      if (error.code === 'EADDRINUSE') {
        logger.error(
          { port },
          'Port is already in use. Please stop the other process or use a different port.',
        )
        process.exit(1)
      } else {
        logger.error({ err: error }, 'Failed to start server')
        process.exit(1)
      }
    })

    // Start memory monitoring (logs startup memory, periodic logging if MEMORY_LOGGING_ENABLED=true)
    memoryMonitor.start()

    logger.info({ host, port }, 'Server listening')
  } catch (err) {
    logger.error({ err }, 'Failed to start server')
    process.exit(1)
  }
}

start()
