import { pino } from 'pino'

// Determine log level: debug in tests, configurable otherwise
const getLogLevel = () => {
  if (process.env.NODE_ENV === 'test' || process.env.VITEST === 'true') {
    return 'debug'
  }
  return process.env.LOG_LEVEL || 'info'
}

// Create the base pino logger with pino-pretty by default, JSON only when explicitly enabled
const pinoLogger = pino({
  level: getLogLevel(),
  transport:
    process.env.USE_JSON_LOGGING === 'true'
      ? undefined
      : {
          target: 'pino-pretty',
          options: {
            colorize: true,
            translateTime: 'HH:MM:ss Z',
            ignore: 'pid,hostname',
            singleLine: false,
          },
        },
})

// Create a wrapper to maintain backward compatibility with existing code
export const logger = {
  trace: (message: string, meta?: any) => pinoLogger.trace(meta, message),
  debug: (message: string, meta?: any) => pinoLogger.debug(meta, message),
  info: (message: string, meta?: any) => pinoLogger.info(meta, message),
  warn: (message: string, meta?: any) => pinoLogger.warn(meta, message),
  error: (message: string, error?: Error | any, meta?: any) => {
    if (error instanceof Error) {
      pinoLogger.error({ ...meta, err: error }, message)
    } else if (error && typeof error === 'object') {
      // If error is actually metadata (no Error passed)
      pinoLogger.error(error, message)
    } else {
      pinoLogger.error(meta, message)
    }
  },
  fatal: (message: string, error?: Error | any, meta?: any) => {
    if (error instanceof Error) {
      pinoLogger.fatal({ ...meta, err: error }, message)
    } else if (error && typeof error === 'object') {
      // If error is actually metadata (no Error passed)
      pinoLogger.fatal(error, message)
    } else {
      pinoLogger.fatal(meta, message)
    }
  },
  child: (bindings: Record<string, any>) => {
    const childPino = pinoLogger.child(bindings)
    return {
      trace: (message: string, meta?: any) => childPino.trace(meta, message),
      debug: (message: string, meta?: any) => childPino.debug(meta, message),
      info: (message: string, meta?: any) => childPino.info(meta, message),
      warn: (message: string, meta?: any) => childPino.warn(meta, message),
      error: (message: string, error?: Error | any, meta?: any) => {
        if (error instanceof Error) {
          childPino.error({ ...meta, err: error }, message)
        } else if (error && typeof error === 'object') {
          childPino.error(error, message)
        } else {
          childPino.error(meta, message)
        }
      },
      fatal: (message: string, error?: Error | any, meta?: any) => {
        if (error instanceof Error) {
          childPino.fatal({ ...meta, err: error }, message)
        } else if (error && typeof error === 'object') {
          childPino.fatal(error, message)
        } else {
          childPino.fatal(meta, message)
        }
      },
    }
  },
}

// Helper function to create request-scoped loggers
export function createRequestLogger(requestId: string, taskId?: string) {
  return logger.child({ requestId, taskId })
}

// Helper function to create task-scoped loggers
export function createTaskLogger(taskId: string) {
  return logger.child({ taskId })
}

// Helper function to create console logger for local development
export function createConsoleLogger(context?: Record<string, any>) {
  return logger.child(context || {})
}
