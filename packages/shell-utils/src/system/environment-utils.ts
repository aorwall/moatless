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
