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
