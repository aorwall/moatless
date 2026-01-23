/**
 * Essential system environment variables safe to pass to child processes.
 * These are needed for basic operation but don't leak sensitive configuration.
 */
const SAFE_SYSTEM_VARS = [
  'PATH',
  'HOME',
  'USER',
  'SHELL',
  'TERM',
  'LANG',
  'LC_ALL',
  'LC_CTYPE',
  'TMPDIR',
  'HOSTNAME',
] as const

/**
 * Version manager variables needed for Node.js/Bun toolchain resolution.
 */
const VERSION_MANAGER_VARS = [
  'NVM_DIR',
  'NVM_BIN',
  'BUN_INSTALL',
  'VOLTA_HOME',
  'FNM_DIR',
  'N_PREFIX',
] as const

/**
 * XDG Base Directory variables for config/cache paths.
 */
const XDG_VARS = ['XDG_CONFIG_HOME', 'XDG_DATA_HOME', 'XDG_CACHE_HOME'] as const

/**
 * CI detection variable.
 */
const CI_VARS = ['CI'] as const

/**
 * All allowed variable names combined.
 */
const ALL_ALLOWED_VARS = [
  ...SAFE_SYSTEM_VARS,
  ...VERSION_MANAGER_VARS,
  ...XDG_VARS,
  ...CI_VARS,
] as const

/**
 * Returns a minimal, safe subset of environment variables from `process.env`.
 *
 * Includes only:
 * - Essential system vars: PATH, HOME, USER, SHELL, TERM, LANG, LC_ALL, LC_CTYPE, TMPDIR, HOSTNAME
 * - Version manager vars: NVM_DIR, NVM_BIN, BUN_INSTALL, VOLTA_HOME, FNM_DIR, N_PREFIX
 * - XDG dirs: XDG_CONFIG_HOME, XDG_DATA_HOME, XDG_CACHE_HOME
 * - CI detection: CI
 *
 * Respects `SHELL_COMMAND_PATH` override for PATH if set.
 */
export function getSafeSystemEnvironment(): Record<string, string> {
  const env: Record<string, string> = {}

  for (const key of ALL_ALLOWED_VARS) {
    const value = process.env[key]
    if (value !== undefined) {
      env[key] = value
    }
  }

  // Respect SHELL_COMMAND_PATH override for PATH
  if (process.env['SHELL_COMMAND_PATH']) {
    env['PATH'] = process.env['SHELL_COMMAND_PATH']
  }

  return env
}
