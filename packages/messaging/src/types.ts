/**
 * Common types shared across messaging implementations
 */

/**
 * Logger interface for messaging services
 */
export interface MessagingLogger {
  debug(obj: object, msg?: string): void
  info(obj: object, msg?: string): void
  warn(obj: object, msg?: string): void
  error(obj: object, msg?: string): void
}

/**
 * Base interface for messaging clients (connection management)
 */
export interface MessagingClient {
  /**
   * Initialize the client (establish connections)
   */
  initialize(): Promise<void>

  /**
   * Health check
   */
  isHealthy(): boolean

  /**
   * Cleanup connections
   */
  close(): Promise<void>
}

/**
 * Default logger that does nothing
 */
export const noopLogger: MessagingLogger = {
  debug: () => undefined,
  info: () => undefined,
  warn: () => undefined,
  error: () => undefined,
}
