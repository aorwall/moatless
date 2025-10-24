import * as os from 'node:os'
import * as path from 'node:path'

/**
 * Get the data directory for the application.
 * Checks environment variable first, then falls back to default location.
 * In tests, this should be set to a temporary directory via environment variable.
 */
export function getDataDir(): string {
  // First check if DATA_DIR is set (typically in tests or production)
  if (process.env['DATA_DIR']) {
    return process.env['DATA_DIR']
  }

  // Default to ~/.moatless in the user's home directory
  return path.join(os.homedir(), '.moatless')
}

/**
 * Get the default data directory (used when DATA_DIR is not set).
 * This is separated for clarity and potential future customization.
 */
export function getDefaultDataDir(): string {
  return path.join(os.homedir(), '.moatless')
}
