import { pino } from 'pino'

// Determine log level: debug in tests, configurable otherwise
const getLogLevel = () => {
  if (process.env.NODE_ENV === 'test' || process.env.VITEST === 'true') {
    return 'debug'
  }
  return process.env.LOG_LEVEL || 'info'
}

// Detect if we're in a production/built environment (either bundled or TypeScript compiled)
const isProductionBuild =
  (typeof __filename !== 'undefined' && __filename.includes('/dist/server.js')) || // Bundled with esbuild
  (typeof __filename !== 'undefined' && __filename.includes('/apps/backend/dist/')) || // TypeScript compiled
  process.env.NODE_ENV === 'production' // Explicit production mode

// Simple pretty formatter for bundled environments
const LEVELS: Record<number, string> = {
  10: 'TRACE',
  20: 'DEBUG',
  30: 'INFO',
  40: 'WARN',
  50: 'ERROR',
  60: 'FATAL',
}

const LEVEL_COLORS: Record<number, string> = {
  10: '\x1b[90m', // TRACE - gray
  20: '\x1b[34m', // DEBUG - blue
  30: '\x1b[32m', // INFO - green
  40: '\x1b[33m', // WARN - yellow
  50: '\x1b[31m', // ERROR - red
  60: '\x1b[35m', // FATAL - magenta
}

const RESET = '\x1b[0m'

function formatTime(timestamp: number) {
  const date = new Date(timestamp)
  return `${date.toTimeString().split(' ')[0]} ${date.toTimeString().split(' ')[1]}`
}

function simplePrettifier() {
  return {
    write(chunk: string) {
      try {
        const obj = JSON.parse(chunk)
        const { level, time, msg, pid, hostname, ...rest } = obj

        const levelLabel = LEVELS[level] || 'UNKNOWN'
        const color = LEVEL_COLORS[level] || ''
        const timeStr = formatTime(time)

        // Format the main log line
        let output = `[${timeStr}] ${color}${levelLabel}${RESET}: ${msg}\n`

        // Add any additional properties
        const extraKeys = Object.keys(rest)
        if (extraKeys.length > 0) {
          const extras = extraKeys.map((key) => `${key}: ${JSON.stringify(rest[key])}`).join(', ')
          output += `    ${extras}\n`
        }

        process.stdout.write(output)
      } catch (_err) {
        // Fallback to raw output if parsing fails
        process.stdout.write(chunk)
      }
    },
  }
}

// Create the base pino logger
const pinoLogger = (() => {
  const baseOptions = {
    level: getLogLevel(),
  }

  if (process.env.USE_JSON_LOGGING === 'true') {
    // Explicit JSON logging
    return pino(baseOptions)
  }
  if (isProductionBuild && process.env.USE_JSON_LOGGING !== 'false') {
    // In production/built environment, use simple prettifier by default
    return pino(baseOptions, simplePrettifier())
  }
  // In development or when explicitly requested, use pino-pretty transport
  return pino({
    ...baseOptions,
    transport: {
      target: 'pino-pretty',
      options: {
        colorize: true,
        translateTime: 'HH:MM:ss Z',
        ignore: 'pid,hostname',
        singleLine: false,
      },
    },
  })
})()

// Create a wrapper to maintain backward compatibility with existing code
export const logger = {
  trace: (message: string, meta?: Record<string, unknown>) => pinoLogger.trace(meta, message),
  debug: (message: string, meta?: Record<string, unknown>) => pinoLogger.debug(meta, message),
  info: (message: string, meta?: Record<string, unknown>) => pinoLogger.info(meta, message),
  warn: (message: string, meta?: Record<string, unknown>) => pinoLogger.warn(meta, message),
  error: (message: string, error?: Error | unknown, meta?: Record<string, unknown>) => {
    if (error instanceof Error) {
      pinoLogger.error({ ...meta, err: error }, message)
    } else if (error && typeof error === 'object') {
      // If error is actually metadata (no Error passed)
      pinoLogger.error(error, message)
    } else {
      pinoLogger.error(meta, message)
    }
  },
  fatal: (message: string, error?: Error | unknown, meta?: Record<string, unknown>) => {
    if (error instanceof Error) {
      pinoLogger.fatal({ ...meta, err: error }, message)
    } else if (error && typeof error === 'object') {
      // If error is actually metadata (no Error passed)
      pinoLogger.fatal(error, message)
    } else {
      pinoLogger.fatal(meta, message)
    }
  },
  child: (bindings: Record<string, unknown>) => {
    const childPino = pinoLogger.child(bindings)
    return {
      trace: (message: string, meta?: Record<string, unknown>) => childPino.trace(meta, message),
      debug: (message: string, meta?: Record<string, unknown>) => childPino.debug(meta, message),
      info: (message: string, meta?: Record<string, unknown>) => childPino.info(meta, message),
      warn: (message: string, meta?: Record<string, unknown>) => childPino.warn(meta, message),
      error: (message: string, error?: Error | unknown, meta?: Record<string, unknown>) => {
        if (error instanceof Error) {
          childPino.error({ ...meta, err: error }, message)
        } else if (error && typeof error === 'object') {
          childPino.error(error, message)
        } else {
          childPino.error(meta, message)
        }
      },
      fatal: (message: string, error?: Error | unknown, meta?: Record<string, unknown>) => {
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

// Helper function to create logger instance (for compatibility with Hono)
export function createLogger() {
  return logger
}
