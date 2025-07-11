import os from 'node:os'
import path from 'node:path'

/**
 * Get the data directory for the application.
 * In tests, this should be set to a temporary directory.
 * In production, it falls back to ~/.moatless or the current working directory.
 */
export function getDataDir(): string {
  // First check if DATA_DIR is set (typically in tests or production)
  if (process.env.DATA_DIR) {
    return process.env.DATA_DIR
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
