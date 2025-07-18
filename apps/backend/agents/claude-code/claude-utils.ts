import { exec } from 'node:child_process'
import { promises as fs } from 'node:fs'
import * as os from 'node:os'
import * as path from 'node:path'
import * as readline from 'node:readline'
import { promisify } from 'node:util'
import type { SystemStatusCheck } from '@moatless/types'

const execAsync = promisify(exec)

export interface ClaudePathInfo {
  binPath: string | null
  configPath: string | null
  isValid: boolean
}

/**
 * Find the Claude binary path by checking various locations
 */
export async function findClaudeBinPath(): Promise<string | null> {
  // First check if it's in PATH
  try {
    const { stdout } = await execAsync('which claude')
    const binPath = stdout.trim()
    if (binPath && (await isValidClaudeBin(binPath))) {
      return binPath
    }
  } catch {
    // Continue to other methods
  }

  // Check common npm/npx installation locations
  const possiblePaths = [
    // Global npm installations
    path.join(os.homedir(), '.npm', 'bin', 'claude'),
    path.join(os.homedir(), '.npm-global', 'bin', 'claude'),
    '/usr/local/bin/claude',
    '/usr/bin/claude',

    // Node.js global module paths
    path.join(os.homedir(), '.local', 'share', 'npm', 'bin', 'claude'),

    // Windows specific paths
    process.platform === 'win32'
      ? path.join(os.homedir(), 'AppData', 'Roaming', 'npm', 'claude.cmd')
      : null,
    process.platform === 'win32'
      ? path.join(os.homedir(), 'AppData', 'Roaming', 'npm', 'claude.exe')
      : null,
  ].filter(Boolean) as string[]

  // Check if npx can find claude
  try {
    const { stdout } = await execAsync('npx --which @anthropic-ai/claude-code')
    const binPath = stdout.trim()
    if (binPath && (await isValidClaudeBin(binPath))) {
      return binPath
    }
  } catch {
    // Continue to other methods
  }

  // Try to find via npm list
  try {
    const { stdout } = await execAsync('npm list -g @anthropic-ai/claude-code --depth=0 --json')
    const result = JSON.parse(stdout)
    if (result.dependencies?.['@anthropic-ai/claude-code']) {
      const packagePath = result.dependencies['@anthropic-ai/claude-code'].path
      if (packagePath) {
        const binPath = path.join(packagePath, 'bin', 'claude')
        if (await isValidClaudeBin(binPath)) {
          return binPath
        }
      }
    }
  } catch {
    // Continue to other methods
  }

  // Check each possible path
  for (const possiblePath of possiblePaths) {
    if (await isValidClaudeBin(possiblePath)) {
      return possiblePath
    }
  }

  return null
}

/**
 * Check if a path is a valid Claude binary
 */
export async function isValidClaudeBin(binPath: string): Promise<boolean> {
  try {
    // Check if file exists and is executable
    await fs.access(binPath, fs.constants.F_OK | fs.constants.X_OK)

    // Try to run claude --version to verify it's working
    const { stdout } = await execAsync(`"${binPath}" --version`, { timeout: 5000 })
    return stdout.includes('claude') || stdout.includes('Claude')
  } catch {
    return false
  }
}

/**
 * Find the Claude config directory
 */
export async function findClaudeConfigPath(): Promise<string | null> {
  const possiblePaths = [
    // Standard XDG config directory
    path.join(os.homedir(), '.config', 'claude'),

    // macOS specific
    path.join(os.homedir(), 'Library', 'Application Support', 'claude'),

    // Windows specific
    process.platform === 'win32' ? path.join(os.homedir(), 'AppData', 'Local', 'claude') : null,
    process.platform === 'win32' ? path.join(os.homedir(), 'AppData', 'Roaming', 'claude') : null,

    // Fallback to home directory
    path.join(os.homedir(), '.claude'),
  ].filter(Boolean) as string[]

  for (const possiblePath of possiblePaths) {
    if (await isValidClaudeConfigPath(possiblePath)) {
      return possiblePath
    }
  }

  return null
}

/**
 * Check if a path is a valid Claude config directory
 */
export async function isValidClaudeConfigPath(configPath: string): Promise<boolean> {
  try {
    // Check if directory exists
    const stats = await fs.stat(configPath)
    if (!stats.isDirectory()) {
      return false
    }

    // Check for common config files
    const configFiles = ['config.json', 'settings.json', 'auth.json']
    for (const configFile of configFiles) {
      try {
        await fs.access(path.join(configPath, configFile), fs.constants.F_OK)
        return true
      } catch {
        // Continue checking other files
      }
    }

    // If directory exists but no config files, it might still be valid
    return true
  } catch {
    return false
  }
}

/**
 * Check if Docker is running
 */
export async function isDockerRunning(): Promise<boolean> {
  try {
    await execAsync('docker info', { timeout: 5000 })
    return true
  } catch {
    return false
  }
}

/**
 * Get Docker version
 */
export async function getDockerVersion(): Promise<string | null> {
  try {
    const { stdout } = await execAsync('docker --version', { timeout: 5000 })
    return stdout.trim()
  } catch {
    return null
  }
}

/**
 * Perform comprehensive system status check
 */
export async function checkSystemStatus(): Promise<SystemStatusCheck> {
  const [binPath, configPath, dockerRunning, dockerVersion] = await Promise.all([
    findClaudeBinPath(),
    findClaudeConfigPath(),
    isDockerRunning(),
    getDockerVersion(),
  ])

  return {
    claude: {
      binPath,
      configPath,
      isValidBin: binPath !== null,
      isValidConfig: configPath !== null,
    },
    docker: {
      isRunning: dockerRunning,
      version: dockerVersion,
    },
  }
}

/**
 * Get standard Claude data directories
 */
export function getClaudeDataPaths(): string[] {
  const homeDir = os.homedir()
  return [
    path.join(homeDir, '.claude', 'projects'),
    path.join(homeDir, '.config', 'claude', 'projects'),
  ]
}

/**
 * Find all .jsonl files in a directory recursively
 * @param dirPath - Directory path to search
 * @param maxAgeMs - Optional maximum age of files in milliseconds (filters by modification time)
 * @returns Array of file paths ordered by modification time (newest first)
 */
export async function findJsonlFiles(dirPath: string, maxAgeMs?: number): Promise<string[]> {
  const filesWithStats: { path: string; mtime: number }[] = []
  const now = Date.now()

  try {
    const items = await fs.readdir(dirPath, { withFileTypes: true, recursive: true })

    for (const item of items) {
      if (item.isFile() && item.name.endsWith('.jsonl')) {
        const fullPath = path.join(item.parentPath || dirPath, item.name)

        try {
          const stats = await fs.stat(fullPath)
          const fileAge = now - stats.mtime.getTime()

          // Check file age if maxAgeMs is specified
          if (maxAgeMs !== undefined && fileAge > maxAgeMs) {
            continue // Skip files older than maxAge
          }

          filesWithStats.push({
            path: fullPath,
            mtime: stats.mtime.getTime(),
          })
        } catch (_statError) {}
      }
    }
  } catch (_error) {
    // Directory doesn't exist or is not accessible
  }

  // Sort by modification time (newest first)
  filesWithStats.sort((a, b) => b.mtime - a.mtime)

  return filesWithStats.map((f) => f.path)
}

/**
 * Parse a JSONL file and return parsed objects
 * @param filePath - Path to the JSONL file
 * @param seenUuids - Optional Set of UUIDs that have already been processed (will be modified)
 * @param sessionIdsToSkip - Optional Set of session IDs to skip entirely
 */
export async function parseJsonlFile<T = any>(
  filePath: string,
  seenUuids?: Set<string>,
  sessionIdsToSkip?: Set<string>,
): Promise<T[]> {
  const entries: T[] = []

  // Extract session ID from filename (format: /path/to/{session-id}.jsonl)
  const filename = path.basename(filePath, '.jsonl')

  // If this session ID should be skipped, return empty array
  if (sessionIdsToSkip?.has(filename)) {
    return entries
  }

  try {
    const fileStream = await fs.open(filePath, 'r')
    const rl = readline.createInterface({
      input: fileStream.createReadStream(),
      crlfDelay: Number.POSITIVE_INFINITY,
    })

    for await (const line of rl) {
      try {
        if (line.trim()) {
          const data = JSON.parse(line)

          // Check if this message has already been processed (by UUID)
          if (seenUuids && data.uuid) {
            if (seenUuids.has(data.uuid)) {
              continue // Skip duplicate message
            }
            seenUuids.add(data.uuid)
          }

          entries.push(data)
        }
      } catch (_parseError) {
        // Skip invalid JSON lines
      }
    }

    await fileStream.close()
  } catch (_error) {
    // File doesn't exist or is not accessible
  }

  return entries
}

/**
 * Read a file and split into lines, filtering out empty lines
 */
export async function readFileLines(filePath: string): Promise<string[]> {
  const fileContent = await fs.readFile(filePath, 'utf-8')
  return fileContent.trim().split('\n').filter(Boolean)
}

/**
 * Check if a file exists and is accessible
 */
export async function fileExists(filePath: string): Promise<boolean> {
  try {
    await fs.access(filePath, fs.constants.F_OK)
    return true
  } catch {
    return false
  }
}
