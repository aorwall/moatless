import { promises as fs } from 'fs'
import path from 'path'
import pino, { type Logger as PinoLogger } from 'pino'

export interface LoggerOptions {
  level?: 'trace' | 'debug' | 'info' | 'warn' | 'error' | 'fatal'
  service?: string
  environment?: string
  logFile?: string
  pretty?: boolean
  console?: boolean
  redact?: string[]
}

export interface LogContext {
  taskId?: string
  userId?: string
  requestId?: string
  executionId?: string
  [key: string]: any
}

class Logger {
  private logger: PinoLogger
  private context: LogContext = {}
  private useConsole: boolean = false

  constructor(options: LoggerOptions = {}) {
    const level = options.level || process.env.LOG_LEVEL || 'info'
    const isDevelopment = process.env.NODE_ENV === 'development'
    this.useConsole = options.console || process.env.USE_CONSOLE_LOGGING === 'true'

    const transport =
      !this.useConsole && (options.pretty || isDevelopment)
        ? {
            target: 'pino-pretty',
            options: {
              colorize: true,
              translateTime: 'HH:MM:ss Z',
              ignore: 'pid,hostname',
              singleLine: false,
            },
          }
        : undefined

    this.logger = pino.default({
      level,
      name: options.service || 'moatless',
      base: {
        environment: options.environment || process.env.NODE_ENV || 'development',
        service: options.service || 'moatless',
      },
      redact: {
        paths: options.redact || [
          'password',
          'token',
          'apiKey',
          'api_key',
          'authorization',
          'cookie',
          'session',
        ],
        censor: '[REDACTED]',
      },
      timestamp: pino.stdTimeFunctions.isoTime,
      transport,
    })

    // Set up file logging if specified
    if (options.logFile) {
      this.setupFileLogging(options.logFile)
    }
  }

  child(context: LogContext): Logger {
    const childLogger = new Logger()
    childLogger.logger = this.logger.child(context)
    childLogger.context = { ...this.context, ...context }
    childLogger.useConsole = this.useConsole
    return childLogger
  }

  setContext(context: LogContext): void {
    this.context = { ...this.context, ...context }
  }

  clearContext(): void {
    this.context = {}
  }

  private formatConsoleLog(level: string, message: string, meta: any = {}): void {
    const timestamp = new Date().toISOString().split('T')[1].replace('Z', '')
    const colors = {
      trace: '\x1b[90m', // gray
      debug: '\x1b[36m', // cyan
      info: '\x1b[32m', // green
      warn: '\x1b[33m', // yellow
      error: '\x1b[31m', // red
      fatal: '\x1b[35m', // magenta
    }
    const reset = '\x1b[0m'
    const color = colors[level as keyof typeof colors] || colors.info

    // Format the main message
    let output = `${color}[${timestamp}] ${level.toUpperCase()}${reset} ${message}`

    // Add context if available
    const allMeta = { ...this.context, ...meta }
    if (Object.keys(allMeta).length > 0) {
      const formattedMeta = Object.entries(allMeta)
        .map(([key, value]) => {
          if (typeof value === 'object' && value !== null) {
            return `${key}: ${JSON.stringify(value, null, 2)}`
          }
          return `${key}: ${value}`
        })
        .join(', ')
      output += ` | ${formattedMeta}`
    }

    console.log(output)
  }

  trace(message: string, meta: any = {}): void {
    if (this.useConsole) {
      this.formatConsoleLog('trace', message, meta)
    } else {
      this.logger.trace({ ...this.context, ...meta }, message)
    }
  }

  debug(message: string, meta: any = {}): void {
    if (this.useConsole) {
      this.formatConsoleLog('debug', message, meta)
    } else {
      this.logger.debug({ ...this.context, ...meta }, message)
    }
  }

  info(message: string, meta: any = {}): void {
    if (this.useConsole) {
      this.formatConsoleLog('info', message, meta)
    } else {
      this.logger.info({ ...this.context, ...meta }, message)
    }
  }

  warn(message: string, meta: any = {}): void {
    if (this.useConsole) {
      this.formatConsoleLog('warn', message, meta)
    } else {
      this.logger.warn({ ...this.context, ...meta }, message)
    }
  }

  error(message: string, error?: Error, meta: any = {}): void {
    const errorMeta = error
      ? {
          error: {
            message: error.message,
            stack: error.stack,
            name: error.name,
          },
        }
      : {}

    if (this.useConsole) {
      this.formatConsoleLog('error', message, { ...errorMeta, ...meta })
    } else {
      this.logger.error({ ...this.context, ...errorMeta, ...meta }, message)
    }
  }

  fatal(message: string, error?: Error, meta: any = {}): void {
    const errorMeta = error
      ? {
          error: {
            message: error.message,
            stack: error.stack,
            name: error.name,
          },
        }
      : {}

    if (this.useConsole) {
      this.formatConsoleLog('fatal', message, { ...errorMeta, ...meta })
    } else {
      this.logger.fatal({ ...this.context, ...errorMeta, ...meta }, message)
    }
  }

  // Request/response logging helpers
  logRequest(req: any, meta: any = {}): void {
    this.info('HTTP Request', {
      method: req.method,
      url: req.url,
      userAgent: req.headers?.['user-agent'],
      ip: req.ip,
      ...meta,
    })
  }

  logResponse(req: any, res: any, responseTime: number, meta: any = {}): void {
    this.info('HTTP Response', {
      method: req.method,
      url: req.url,
      statusCode: res.statusCode,
      responseTime: `${responseTime}ms`,
      ...meta,
    })
  }

  // Performance logging
  logPerformance(operation: string, duration: number, meta: any = {}): void {
    this.info('Performance', {
      operation,
      duration: `${duration}ms`,
      ...meta,
    })
  }

  // Business logic logging
  logTaskEvent(taskId: string, event: string, meta: any = {}): void {
    this.info('Task Event', {
      taskId,
      event,
      ...meta,
    })
  }

  logAgentEvent(taskId: string, agentType: string, event: string, meta: any = {}): void {
    this.info('Agent Event', {
      taskId,
      agentType,
      event,
      ...meta,
    })
  }

  logWorkspaceEvent(taskId: string, workspaceType: string, event: string, meta: any = {}): void {
    this.info('Workspace Event', {
      taskId,
      workspaceType,
      event,
      ...meta,
    })
  }

  private async setupFileLogging(logFile: string): Promise<void> {
    try {
      await fs.mkdir(path.dirname(logFile), { recursive: true })

      // Add file stream destination
      const fileStream = pino.destination({
        dest: logFile,
        sync: false,
        mkdir: true,
      })

      // Create multi-stream logger
      const streams = [{ stream: fileStream }]

      if (process.env.NODE_ENV === 'development') {
        streams.push({
          stream: pino.transport({
            target: 'pino-pretty',
            options: {
              colorize: true,
              translateTime: 'HH:MM:ss Z',
            },
          }),
        })
      }

      this.logger = pino.default(
        {
          level: this.logger.level,
        },
        pino.multistream(streams),
      )
    } catch (error) {
      this.error('Failed to setup file logging', error as Error)
    }
  }
}

// Create default logger instance
export const logger = new Logger({
  service: 'moatless-server',
  environment: process.env.NODE_ENV,
  logFile: process.env.LOG_FILE,
  console: process.env.USE_CONSOLE_LOGGING === 'true',
})

// Export logger class for custom instances
export { Logger }

// Helper function to create request-scoped loggers
export function createRequestLogger(requestId: string, taskId?: string): Logger {
  return logger.child({ requestId, taskId })
}

// Helper function to create task-scoped loggers
export function createTaskLogger(taskId: string): Logger {
  return logger.child({ taskId })
}

// Helper function to create console logger for local development
export function createConsoleLogger(context?: LogContext): Logger {
  return new Logger({
    console: true,
    level: 'debug',
  }).child(context || {})
}
