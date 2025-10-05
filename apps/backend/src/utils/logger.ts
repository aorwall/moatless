import type { Logger as PinoLogger } from 'pino'
import { pino } from 'pino'

// Determine log level: debug in tests, configurable otherwise
const getLogLevel = () => {
  if (process.env['NODE_ENV'] === 'test' || process.env['VITEST'] === 'true') {
    return 'debug'
  }
  return process.env['LOG_LEVEL'] || 'info'
}

// Detect if we're in a production/built environment
const isProductionBuild =
  (typeof __filename !== 'undefined' && __filename.includes('/dist/server.js')) ||
  (typeof __filename !== 'undefined' && __filename.includes('/apps/backend/dist/')) ||
  process.env['NODE_ENV'] === 'production'

// Create the base pino logger with appropriate formatting
const pinoLogger = (() => {
  const baseOptions = {
    level: getLogLevel(),
  }

  if (process.env['USE_JSON_LOGGING'] === 'true') {
    // Explicit JSON logging
    return pino(baseOptions)
  }
  if (isProductionBuild && process.env['USE_JSON_LOGGING'] !== 'false') {
    // In production, use JSON by default
    return pino(baseOptions)
  }
  // In development, use pino-pretty transport
  return pino({
    ...baseOptions,
    transport: {
      target: 'pino-pretty',
      options: {
        colorize: true,
        translateTime: 'HH:MM:ss Z',
        singleLine: false,
        ignore: '', // Show all fields including pid and hostname
        levelFirst: true, // Display level before timestamp
      },
    },
  })
})()

// Wrapper type that extends Pino's Logger for compatibility
type LogMethod = {
  // Support both old API (message, meta) and Pino native API (meta, message)
  (message: string, meta?: Record<string, unknown>): void
  (meta: Record<string, unknown>, message: string): void
}

type ErrorLogMethod = {
  // Support old API with error parameter
  (message: string, error?: Error | unknown, meta?: Record<string, unknown>): void
  // Support Pino native API
  (meta: Record<string, unknown>, message: string): void
}

export interface Logger
  extends Omit<PinoLogger, 'trace' | 'debug' | 'info' | 'warn' | 'error' | 'fatal' | 'child'> {
  trace: LogMethod
  debug: LogMethod
  info: LogMethod
  warn: LogMethod
  error: ErrorLogMethod
  fatal: ErrorLogMethod
  child: (bindings: Record<string, unknown>) => Logger
}

// Create wrapper that supports both API styles
function wrapLogger(pinoInstance: PinoLogger): Logger {
  const wrapper = {
    // Delegate all BaseLogger properties to the pino instance
    get level() {
      return pinoInstance.level
    },
    set level(value: string) {
      pinoInstance.level = value
    },
    get silent() {
      return pinoInstance.silent
    },

    trace: ((arg1: string | Record<string, unknown>, arg2?: string | Record<string, unknown>) => {
      if (typeof arg1 === 'string') {
        // Old API: (message, meta)
        pinoInstance.trace((arg2 as Record<string, unknown>) || {}, arg1)
      } else {
        // Pino native API: (meta, message)
        pinoInstance.trace(arg1, arg2 as string)
      }
    }) as LogMethod,

    debug: ((arg1: string | Record<string, unknown>, arg2?: string | Record<string, unknown>) => {
      if (typeof arg1 === 'string') {
        pinoInstance.debug((arg2 as Record<string, unknown>) || {}, arg1)
      } else {
        pinoInstance.debug(arg1, arg2 as string)
      }
    }) as LogMethod,

    info: ((arg1: string | Record<string, unknown>, arg2?: string | Record<string, unknown>) => {
      if (typeof arg1 === 'string') {
        pinoInstance.info((arg2 as Record<string, unknown>) || {}, arg1)
      } else {
        pinoInstance.info(arg1, arg2 as string)
      }
    }) as LogMethod,

    warn: ((arg1: string | Record<string, unknown>, arg2?: string | Record<string, unknown>) => {
      if (typeof arg1 === 'string') {
        pinoInstance.warn((arg2 as Record<string, unknown>) || {}, arg1)
      } else {
        pinoInstance.warn(arg1, arg2 as string)
      }
    }) as LogMethod,

    error: ((
      arg1: string | Record<string, unknown>,
      arg2?: Error | unknown | string | Record<string, unknown>,
      arg3?: Record<string, unknown>,
    ) => {
      if (typeof arg1 === 'string') {
        // Old API: (message, error?, meta?)
        if (arg2 instanceof Error) {
          pinoInstance.error({ ...arg3, err: arg2 }, arg1)
        } else if (arg2 && typeof arg2 === 'object') {
          pinoInstance.error(arg2, arg1)
        } else {
          pinoInstance.error(arg3 || {}, arg1)
        }
      } else {
        // Pino native API: (meta, message)
        pinoInstance.error(arg1, arg2 as string)
      }
    }) as ErrorLogMethod,

    fatal: ((
      arg1: string | Record<string, unknown>,
      arg2?: Error | unknown | string | Record<string, unknown>,
      arg3?: Record<string, unknown>,
    ) => {
      if (typeof arg1 === 'string') {
        if (arg2 instanceof Error) {
          pinoInstance.fatal({ ...arg3, err: arg2 }, arg1)
        } else if (arg2 && typeof arg2 === 'object') {
          pinoInstance.fatal(arg2, arg1)
        } else {
          pinoInstance.fatal(arg3 || {}, arg1)
        }
      } else {
        pinoInstance.fatal(arg1, arg2 as string)
      }
    }) as ErrorLogMethod,

    child: (bindings: Record<string, unknown>) => wrapLogger(pinoInstance.child(bindings)),
  } as Logger

  return wrapper
}

// Export wrapped logger for backward compatibility
export const logger = wrapLogger(pinoLogger)

// Helper function to create task-scoped loggers (returns wrapped logger)
export function createTaskLogger(taskId: string): Logger {
  return wrapLogger(pinoLogger.child({ taskId }))
}

// Helper function to create logger instance (for compatibility with Hono)
export function createLogger() {
  return logger
}
