import os from 'node:os'
import path from 'node:path'
import { getSettingsService } from '../services/settings.service.js'

/**
 * Get the data directory for the application.
 * Uses the settings service to get the configured data directory.
 * In tests, this should be set to a temporary directory via environment variable.
 */
export function getDataDir(): string {
  try {
    // Try to get from settings service
    const settingsService = getSettingsService()
    return settingsService.getDataDir()
  } catch {
    // Fallback if settings service is not initialized
    // First check if DATA_DIR is set (typically in tests or production)
    if (process.env.DATA_DIR) {
      return process.env.DATA_DIR
    }

    // Default to ~/.moatless in the user's home directory
    return path.join(os.homedir(), '.moatless')
  }
}

/**
 * Get the default data directory (used when DATA_DIR is not set).
 * This is separated for clarity and potential future customization.
 */
export function getDefaultDataDir(): string {
  return path.join(os.homedir(), '.moatless')
}
