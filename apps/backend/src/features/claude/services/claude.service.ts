import { existsSync, promises as fs } from 'node:fs'
import * as os from 'node:os'
import * as path from 'node:path'
import type { AgentMessage, RepositoryConfig, Task } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { SettingsService } from '../../../services/settings.service'
import { logger } from '../../../utils/logger'
import { findJsonlFiles } from '../agent/claude-utils'

@injectable()
export class ClaudeService {
  private messageFilePathCache: Map<string, string> = new Map()

  constructor(@inject(SettingsService) private settingsService: SettingsService) {}

  // ===============================
  // Private Helper Methods
  // ===============================

  /**
   * Get Claude projects directory path
   */
  public getClaudeProjectPath(): string {
    const claudeSettings = this.settingsService.getClaudeSettings()

    // Use configDir from settings if available, otherwise use default
    const claudeConfigDir = claudeSettings.configDir || path.join(os.homedir(), '.claude')
    return path.join(claudeConfigDir, 'projects')
  }

  /**
   * Read Claude config.json file and return as object
   */
  public async readClaudeConfig(): Promise<Record<string, unknown>> {
    const claudeSettings = this.settingsService.getClaudeSettings()
    const claudeConfigDir = claudeSettings.configDir || path.join(os.homedir(), '.claude')
    const configPath = path.join(claudeConfigDir, '.config.json')

    try {
      const configContent = await fs.readFile(configPath, 'utf-8')
      return JSON.parse(configContent)
    } catch (error) {
      if (error instanceof Error && 'code' in error && error.code === 'ENOENT') {
        throw new Error(`Claude config.json file does not exist on path ${configPath}`)
      }
      logger.error('Error reading Claude config.json', error as Error, {
        configPath,
      })
      throw error
    }
  }

  /**
   * Get all project directories from Claude config
   */
  public async getClaudeProjectDirectories(): Promise<string[]> {
    const config = await this.readClaudeConfig()

    if (!config['projects'] || typeof config['projects'] !== 'object') {
      logger.warn('Claude config.json does not contain projects object', {
        config,
      })
      return []
    }

    // Extract all keys from the projects object, which are the file paths
    return Object.keys(config['projects'])
  }

  /**
   * Generate project ID from path, using repository path for docker environments
   */
  public generateProjectId(task: Task, repository: RepositoryConfig): string {
    // For docker environments, use repository path if available (local/template repos)
    logger.info('Generating project ID', { repository })
    const executionMode = this.settingsService.getWorkspaceExecutionMode()
    if (executionMode === 'docker' && repository.kind === 'local') {
      return repository.path.replace(/[^a-zA-Z0-9]/g, '-')
    }

    // For local environments or remote repositories, use workspace path
    const workspacePath = task.workspacePath
    if (!workspacePath) {
      throw new Error('No workspace path available for generating project ID')
    }

    return workspacePath.replace(/[^a-zA-Z0-9]/g, '-')
  }

  /**
   * Find messages file by searching for {agentSessionId}.jsonl in any subdirectory
   */
  private async findMessagesFile(agentSessionId: string): Promise<string | null> {
    // Check cache first
    const cached = this.messageFilePathCache.get(agentSessionId)
    if (cached) {
      // Verify the file still exists
      try {
        await fs.access(cached, fs.constants.F_OK)
        return cached
      } catch {
        // File no longer exists, remove from cache
        this.messageFilePathCache.delete(agentSessionId)
      }
    }

    // Search for the file
    const claudeProjectPath = this.getClaudeProjectPath()
    const targetFileName = `${agentSessionId}.jsonl`

    try {
      const allFiles = await findJsonlFiles(claudeProjectPath)

      for (const filePath of allFiles) {
        if (path.basename(filePath) === targetFileName) {
          // Cache the result
          this.messageFilePathCache.set(agentSessionId, filePath)
          return filePath
        }
      }
    } catch (error) {
      logger.error('Error searching for messages file', error as Error, {
        agentSessionId,
        claudeProjectPath,
      })
    }

    return null
  }

  /**
   * Get Claude messages file path for a specific agent session
   */
  public async getClaudeMessagesFilePath(
    agentSessionId: string,
    projectId?: string,
  ): Promise<string> {
    // Try to find the file first
    const foundPath = await this.findMessagesFile(agentSessionId)
    if (foundPath) {
      return foundPath
    }

    // If not found and projectId is provided, return the expected path
    if (projectId) {
      const claudeProjectPath = this.getClaudeProjectPath()
      return path.join(claudeProjectPath, projectId, `${agentSessionId}.jsonl`)
    }

    // If no projectId and file not found, return a default path
    const claudeProjectPath = this.getClaudeProjectPath()
    return path.join(claudeProjectPath, `${agentSessionId}.jsonl`)
  }

  /**
   * Read Claude messages for a specific agent session
   */
  public async readClaudeMessages(agentSessionId: string): Promise<AgentMessage[]> {
    const messages: AgentMessage[] = []

    // Try to find the session file
    const filePath = await this.findMessagesFile(agentSessionId)

    if (!filePath) {
      logger.debug('No Claude messages file found for session', {
        agentSessionId,
      })
      return messages
    }

    try {
      const fileContent = await fs.readFile(filePath, 'utf-8')
      const lines = fileContent.trim().split('\n').filter(Boolean)

      for (const line of lines) {
        try {
          const agentMessage = JSON.parse(line) as AgentMessage

          // Transform ExitPlanMode tool calls to regular text messages
          const transformedMessage = this.transformExitPlanModeMessage(agentMessage)
          messages.push(transformedMessage)
        } catch (parseError) {
          logger.warn('Failed to parse Claude agent message', {
            agentSessionId,
            error: parseError instanceof Error ? parseError.message : String(parseError),
          })
        }
      }

      logger.debug('Read Claude messages from file', {
        agentSessionId,
        filePath,
        messageCount: messages.length,
      })
    } catch (error) {
      if (error instanceof Error && 'code' in error && error.code === 'ENOENT') {
        logger.debug('Claude messages file does not exist', {
          agentSessionId,
          filePath,
        })
      } else {
        logger.error('Error reading Claude messages file', error as Error, {
          agentSessionId,
          filePath,
        })
      }
    }

    return messages
  }

  /**
   * Transform ExitPlanMode tool calls into regular text messages
   */
  private transformExitPlanModeMessage(message: AgentMessage): AgentMessage {
    // Only process assistant messages
    if (message.type !== 'assistant') {
      return message
    }

    // Check if message has content
    if (
      !(
        'message' in message &&
        message.message &&
        'content' in message.message &&
        message.message.content
      )
    ) {
      return message
    }

    const content = Array.isArray(message.message.content)
      ? message.message.content
      : [message.message.content]
    let hasExitPlanMode = false
    let extractedPlan: string | null = null

    // Look for ExitPlanMode tool calls
    for (const contentItem of content) {
      if (
        contentItem &&
        typeof contentItem === 'object' &&
        'type' in contentItem &&
        contentItem.type === 'tool_use' &&
        'name' in contentItem &&
        contentItem.name === 'ExitPlanMode' &&
        'input' in contentItem &&
        contentItem.input &&
        typeof contentItem.input === 'object' &&
        'plan' in contentItem.input
      ) {
        hasExitPlanMode = true
        extractedPlan = contentItem.input['plan'] as string
        logger.debug('Transforming ExitPlanMode tool call to text message', {
          planLength: extractedPlan?.length,
        })
        break
      }
    }

    // Transform the message if ExitPlanMode was found
    if (hasExitPlanMode && extractedPlan) {
      return {
        ...message,
        message: {
          ...message.message,
          content: [
            {
              type: 'text',
              text: extractedPlan,
            },
          ],
        },
      }
    }

    return message
  }

  /**
   * Find Claude bin directory by checking common locations
   */
  findClaudeBinDir(): string {
    const possiblePaths = this.buildPossibleClaudePaths()
    return this.searchForClaudeExecutable(possiblePaths)
  }

  private buildPossibleClaudePaths(): string[] {
    return [
      ...this.getSystemBinPaths(),
      ...this.getNvmBinPath(),
      ...this.getNpmGlobalPaths(),
      ...this.getUserSpecificPaths(),
      ...this.getSystemPathDirs(),
    ]
  }

  private getSystemBinPaths(): string[] {
    return [
      '/usr/bin', // Common system location
      '/usr/local/bin',
    ]
  }

  private getNvmBinPath(): string[] {
    return process.env['NVM_BIN'] ? [process.env['NVM_BIN']] : []
  }

  private getNpmGlobalPaths(): string[] {
    const npmPrefixes = [
      path.join(os.homedir(), '.npm-global'),
      path.join(os.homedir(), '.nvm', 'versions', 'node'),
      '/usr/local',
    ]

    return npmPrefixes.flatMap((prefix) => [
      ...this.getNpmBinPath(prefix),
      ...this.getNvmVersionPaths(prefix),
    ])
  }

  private getNpmBinPath(prefix: string): string[] {
    const npmBinPath = path.join(prefix, 'bin')
    return existsSync(npmBinPath) ? [npmBinPath] : []
  }

  private getNvmVersionPaths(prefix: string): string[] {
    if (!prefix.includes('.nvm')) {
      return []
    }

    try {
      const versionDirs = require('node:fs').readdirSync(path.join(prefix))
      return versionDirs
        .map((version: string) => path.join(prefix, version, 'bin'))
        .filter((versionBinPath: string) => existsSync(versionBinPath))
    } catch {
      // Ignore errors reading nvm directories
      return []
    }
  }

  private getUserSpecificPaths(): string[] {
    return [path.join(os.homedir(), '.local', 'bin'), path.join(os.homedir(), 'bin')]
  }

  private getSystemPathDirs(): string[] {
    if (!process.env['PATH']) {
      return []
    }
    return process.env['PATH'].split(path.delimiter)
  }

  private searchForClaudeExecutable(possiblePaths: string[]): string {
    for (const dir of possiblePaths) {
      const claudePath = path.join(dir, 'claude')
      if (existsSync(claudePath)) {
        return dir
      }
    }
    return ''
  }

  /**
   * Find Claude config directory
   */
  findClaudeConfigDir(): string {
    // First check CLAUDE_CONFIG_DIR env var
    if (process.env['CLAUDE_CONFIG_DIR'] && existsSync(process.env['CLAUDE_CONFIG_DIR'])) {
      return process.env['CLAUDE_CONFIG_DIR']
    }

    // Check default location
    const defaultConfigDir = path.join(os.homedir(), '.claude')
    if (existsSync(defaultConfigDir)) {
      return defaultConfigDir
    }

    // Return default even if it doesn't exist yet
    return defaultConfigDir
  }

  /**
   * Validate Claude paths and return their validity status
   */
  async validateClaudePaths(
    binDir: string,
    configDir: string,
  ): Promise<{
    binDirValid: boolean
    configDirValid: boolean
    claudeVersion: string | undefined
    error: string | undefined
  }> {
    const { binDirValid, claudeVersion, error } = await this.validateBinDirectory(binDir)
    const configDirValid = this.validateConfigDirectory(configDir)

    return {
      binDirValid,
      configDirValid,
      claudeVersion,
      error,
    }
  }

  private async validateBinDirectory(binDir: string): Promise<{
    binDirValid: boolean
    claudeVersion: string | undefined
    error: string | undefined
  }> {
    if (!binDir) {
      return { binDirValid: false, claudeVersion: undefined, error: undefined }
    }

    const claudeBinPath = path.join(binDir, 'claude')

    if (!existsSync(claudeBinPath)) {
      return {
        binDirValid: false,
        claudeVersion: undefined,
        error: `Claude binary not found at ${claudeBinPath}`,
      }
    }

    return await this.validateClaudeBinary(claudeBinPath)
  }

  private async validateClaudeBinary(claudeBinPath: string): Promise<{
    binDirValid: boolean
    claudeVersion: string | undefined
    error: string | undefined
  }> {
    try {
      const { stdout, stderr } = await this.executeClaudeVersion(claudeBinPath)
      return this.processClaudeVersionOutput(stdout, stderr, claudeBinPath)
    } catch (execError) {
      return this.handleClaudeExecutionError(execError, claudeBinPath)
    }
  }

  private async executeClaudeVersion(
    claudeBinPath: string,
  ): Promise<{ stdout: string; stderr: string }> {
    const { exec } = await import('node:child_process')
    const { promisify } = await import('node:util')
    const execAsync = promisify(exec)

    return await execAsync(`"${claudeBinPath}" --version`, {
      timeout: 5000,
      env: {
        ...process.env,
      },
    })
  }

  private processClaudeVersionOutput(
    stdout: string,
    stderr: string,
    claudeBinPath: string,
  ): {
    binDirValid: boolean
    claudeVersion: string | undefined
    error: string | undefined
  } {
    if (stdout) {
      const claudeVersion = stdout.trim()
      logger.info('Claude binary validated successfully', {
        claudeBinPath,
        version: claudeVersion,
      })
      return { binDirValid: true, claudeVersion, error: undefined }
    }

    if (stderr) {
      const error = `Claude binary error: ${stderr}`
      logger.error('Claude binary failed validation', new Error(stderr), {
        claudeBinPath,
      })
      return { binDirValid: false, claudeVersion: undefined, error }
    }

    return { binDirValid: false, claudeVersion: undefined, error: undefined }
  }

  private handleClaudeExecutionError(
    execError: unknown,
    claudeBinPath: string,
  ): {
    binDirValid: boolean
    claudeVersion: string | undefined
    error: string | undefined
  } {
    const error = `Failed to execute claude --version: ${execError instanceof Error ? execError.message : String(execError)}`
    logger.error('Failed to validate claude binary', execError as Error, {
      claudeBinPath,
      error,
    })
    return { binDirValid: false, claudeVersion: undefined, error }
  }

  private validateConfigDirectory(configDir: string): boolean {
    return configDir ? existsSync(configDir) : false
  }
}
