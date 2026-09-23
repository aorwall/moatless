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

export const SANDBOX_HOME = '/home/moatless'

/** Cache directory for Claude CLI and MCP logs. */
export const SANDBOX_CACHE_DIR = '/home/moatless/.cache'

export const SANDBOX_CLAUDE_CONFIG_DIR = '/opt/moatless/.claude'

export const SANDBOX_WORKSPACE_DIR = '/opt/moatless/repo'

export const SANDBOX_DATA_DIR = '/opt/moatless/data'

/** @deprecated Files are downloaded on demand through the backend API. */
export const SANDBOX_UPLOAD_DIR = '/opt/moatless/workspace/uploads'

/**
 * Downloads directory for files fetched via backend API
 */
export const SANDBOX_DOWNLOADS_DIR = '/opt/moatless/downloads'

/**
 * Plugins directory, cloned at sandbox start. Under the workspace so the
 * workspace snapshot restores it after a pod replacement.
 */
export const SANDBOX_PLUGINS_DIR = '/opt/moatless/workspace/.plugins'

export const SANDBOX_CLAUDE_EXECUTABLE_PATH = '/usr/local/bin/claude'
