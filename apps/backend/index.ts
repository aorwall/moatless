import { serve } from '@hono/node-server'
import { config } from 'dotenv'
import { createApp } from './app.js'
import { createLogger } from './utils/logger.js'

config() // Load environment variables

const logger = createLogger()

const start = async () => {
  try {
    const app = await createApp()

    const port = parseInt(process.env.PORT || '8000', 10)
    const host = process.env.HOST || '0.0.0.0'

    // Setup graceful shutdown handlers
    const gracefulShutdown = async (signal: string) => {
      logger.info(`Received ${signal}, starting graceful shutdown...`)

      try {
        // Get services from the app context - we can't access them from Hono app directly
        // For now, just log that we're shutting down
        logger.info('Shutting down services...')

        // TODO: Implement proper service shutdown when we can access services from Hono app

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

    serve({
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
