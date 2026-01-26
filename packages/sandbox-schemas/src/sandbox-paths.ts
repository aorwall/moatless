/**
 * Sandbox Path Constants
 *
 * Defines standard paths used in the sandbox container environment.
 * These paths are shared between backend (for Docker container setup)
 * and sandbox (for runtime execution).
 *
 * Path Structure (following Linux FHS):
 * - HOME: /home/moatless (standard Linux home directory)
 * - CACHE_DIR: /home/moatless/.cache (user cache, ephemeral)
 * - CLAUDE_CONFIG_DIR: /opt/moatless/.claude (Claude Code configuration, may be volume-mounted)
 * - WORKSPACE_DIR: /opt/moatless/repo (cloned repository workspace)
 * - DATA_DIR: /opt/moatless/data (persistent data storage)
 */

/**
 * Sandbox user home directory (standard Linux location)
 */
export const SANDBOX_HOME = '/home/moatless'

/**
 * Cache directory for Claude CLI and MCP logs (in user home per Linux FHS)
 */
export const SANDBOX_CACHE_DIR = '/home/moatless/.cache'

/**
 * Claude Code configuration directory (inside /opt/moatless for persistence)
 */
export const SANDBOX_CLAUDE_CONFIG_DIR = '/opt/moatless/.claude'

/**
 * Repository workspace directory (where code is cloned)
 */
export const SANDBOX_WORKSPACE_DIR = '/opt/moatless/repo'

/**
 * Data directory for persistent storage
 */
export const SANDBOX_DATA_DIR = '/opt/moatless/data'

/**
 * Plugins directory (mounted from shared storage, inside Claude config dir for auto-discovery)
 */
export const SANDBOX_PLUGINS_DIR = `${SANDBOX_CLAUDE_CONFIG_DIR}/plugins`

/**
 * Claude binary directory (global installation)
 */
export const SANDBOX_CLAUDE_BIN_DIR = '/usr/bin'

/**
 * All sandbox paths as a single object
 */
export const SANDBOX_PATHS = {
  HOME: SANDBOX_HOME,
  CACHE_DIR: SANDBOX_CACHE_DIR,
  CLAUDE_CONFIG_DIR: SANDBOX_CLAUDE_CONFIG_DIR,
  WORKSPACE_DIR: SANDBOX_WORKSPACE_DIR,
  DATA_DIR: SANDBOX_DATA_DIR,
  PLUGINS_DIR: SANDBOX_PLUGINS_DIR,
  CLAUDE_BIN_DIR: SANDBOX_CLAUDE_BIN_DIR,
} as const
