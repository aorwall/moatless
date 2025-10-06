import type { RepositoryConfig } from '@moatless/types'

/**
 * Get a safe timeout value with bounds checking
 * @param timeout User-provided timeout
 * @param defaultTimeout Default timeout if not provided (default: 30s)
 * @param maxTimeout Maximum allowed timeout (default: 10min)
 * @returns Safe timeout value between 1s and maxTimeout
 */
export function getSafeTimeout(
  timeout?: number,
  defaultTimeout = 30000,
  maxTimeout = 600000,
): number {
  if (!timeout) {
    return defaultTimeout
  }
  return Math.min(Math.max(timeout, 1000), maxTimeout) // Min 1s, max 10min
}

/**
 * Create environment variables object from configuration
 * @param baseEnvVars Base environment variables
 * @param repositoryEnvVars Repository-specific environment variables
 * @returns Merged environment variables
 */
export function createEnvironmentVariables(
  baseEnvVars: Record<string, string>,
  repositoryEnvVars?: Record<string, string>,
): Record<string, string> {
  return {
    ...baseEnvVars,
    ...(repositoryEnvVars || {}),
  }
}

/**
 * Format command string for safe execution in containers
 * @param command Command to format
 * @returns Formatted command with escaped quotes
 */
export function formatContainerCommand(command: string): string {
  // Escape double quotes in the command
  return command.replace(/"/g, '\\"')
}

/**
 * Validate repository configuration
 * @param config Repository configuration to validate
 * @throws Error if configuration is invalid
 */
export function validateRepositoryConfig(config: RepositoryConfig): void {
  if (!config.id) {
    throw new Error('Repository configuration must have an ID')
  }
  if (!config.path) {
    throw new Error('Repository configuration must have a path')
  }
  if (!config.codeExecution) {
    throw new Error('Repository configuration must have code execution settings')
  }
}

/**
 * Handle background task execution with proper error handling
 * @param tasks Array of tasks to execute
 * @param onError Optional error handler for individual task failures
 */
export async function handleBackgroundTasks(
  tasks: (() => Promise<void>)[],
  onError?: (error: Error, taskIndex: number) => Promise<void>,
): Promise<void> {
  const promises = tasks.map(async (task, index) => {
    try {
      await task()
    } catch (error) {
      if (onError) {
        await onError(error as Error, index)
      } else {
        throw error
      }
    }
  })

  await Promise.all(promises)
}

/**
 * Extract error message from unknown error type
 * @param error Unknown error
 * @returns Error message string
 */
export function getErrorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message
  }
  if (typeof error === 'string') {
    return error
  }
  return String(error)
}

/**
 * Check if a command is safe to execute
 * @param command Command to check
 * @returns True if command appears safe
 */
export function isCommandSafe(command: string): boolean {
  // Check for dangerous patterns
  const dangerousPatterns = [
    /rm\s+-rf\s+\//, // rm -rf /
    /:\(\)\s*\{[^}]*:\|:&\s*\};:/, // Fork bomb
    />\s*\/dev\/sda/, // Direct disk write
    /dd\s+if=.*of=\/dev\//, // dd to device
  ]

  return !dangerousPatterns.some((pattern) => pattern.test(command))
}

/**
 * Normalize server commands to array format
 * @param commands Single command or array of commands
 * @returns Array of commands with empty strings filtered out
 */
export function normalizeServerCommands(commands: string | string[]): string[] {
  const commandsArray = Array.isArray(commands) ? commands : [commands]
  return commandsArray.filter((cmd) => cmd.trim() !== '')
}

/**
 * Extract safe system environment variables for child processes
 * Returns only essential system variables, excluding backend-specific variables
 * @returns Object with safe environment variables
 */
export function getSafeSystemEnvironment(): Record<string, string> {
  const safeVars = [
    // Essential system paths
    'PATH',
    'HOME',
    'USER',
    'TMPDIR',
    'TEMP',
    'TMP',

    // Shell and terminal
    'SHELL',
    'TERM',
    'LANG',
    'LC_ALL',
    'LC_CTYPE',

    // Development tools (version managers, package managers)
    'NVM_DIR',
    'NVM_BIN',
    'NVM_INC',
    'PYENV_ROOT',
    'RBENV_ROOT',
    'SDKMAN_DIR',

    // Platform-specific
    'PLATFORM',
    'OS',
    'OSTYPE',

    // CI/CD detection (useful for conditional behavior)
    'CI',
    'CONTINUOUS_INTEGRATION',
  ]

  const safeEnv: Record<string, string> = {}

  for (const key of safeVars) {
    const value = process.env[key]
    if (value !== undefined) {
      safeEnv[key] = value
    }
  }

  return safeEnv
}
