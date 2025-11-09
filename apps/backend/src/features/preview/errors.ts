/**
 * Preview feature error classes
 */

/**
 * Error class for Playwright-related errors
 * Provides error chaining support via cause property
 */
export class PlaywrightError extends Error {
  constructor(
    message: string,
    public override readonly cause?: Error,
  ) {
    super(message)
    this.name = 'PlaywrightError'

    // Maintain proper stack trace for where error was thrown (V8 only)
    if (Error.captureStackTrace) {
      Error.captureStackTrace(this, PlaywrightError)
    }
  }
}

/**
 * Extract detailed error message from error chain
 * Builds a message that includes both high-level context and root cause
 *
 * @param error - The error to extract message from
 * @param context - Optional high-level context (e.g., "Failed to navigate to /about")
 * @returns Formatted error message with root cause
 *
 * @example
 * // With PlaywrightError chain
 * extractErrorMessage(playwrightError, "Failed to navigate")
 * // Returns: "Failed to navigate: Page timeout: Navigation exceeded 30s"
 *
 * @example
 * // With simple Error
 * extractErrorMessage(new Error("Connection refused"))
 * // Returns: "Connection refused"
 */
export function extractErrorMessage(error: unknown, context?: string): string {
  if (!error) {
    return context || 'Unknown error'
  }

  // Build array of error messages from error chain
  const messages: string[] = []

  // Add context if provided
  if (context) {
    messages.push(context)
  }

  // Extract messages from error chain
  let currentError: Error | undefined = error instanceof Error ? error : new Error(String(error))

  while (currentError) {
    // Add current error message if not already in context
    const currentMessage = currentError.message
    if (currentMessage && !messages.includes(currentMessage)) {
      messages.push(currentMessage)
    }

    // Follow the cause chain
    currentError =
      'cause' in currentError && currentError.cause instanceof Error
        ? currentError.cause
        : undefined
  }

  // Join messages with ': ' separator, avoiding duplicates
  return messages.join(': ')
}
