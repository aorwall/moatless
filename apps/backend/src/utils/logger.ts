import { type Logger, pino } from 'pino'

export type { Logger }

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

// Format level as text string instead of numeric for Loki/Grafana compatibility
const formatters = {
  level(label: string) {
    return { level: label }
  },
}

// Create the pino logger with appropriate formatting
// Using 'as any as Logger' to work around TypeScript's strict generic checking with pino
// The logger is functionally compatible with all pino Logger types at runtime
export const logger = (() => {
  const baseOptions = {
    level: getLogLevel(),
    formatters,
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
  // biome-ignore lint/suspicious/noExplicitAny: Type assertion needed to work around TypeScript's strict generic checking with pino
})() as any as Logger
