import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { RepositoryNotFoundError, ValidationError } from '@shared/constants/errors.js'
import type { FastifyInstance } from 'fastify'
import type {
  CreateRepositoryRequest,
  FileSearchResponse,
  PreviewConfig,
  Repository,
  RepositoryConfig,
  SetupScriptResponse,
} from '../../shared/types/repository.js'
import { searchFilesInDirectoryNew } from '../utils/file-search.js'
import { logger } from '../utils/logger.js'
import type { StorageBackend } from './storage.js'

export class RepositoryService {
  constructor(private storage: StorageBackend) {}

  async createRepository(request: CreateRepositoryRequest): Promise<RepositoryConfig> {
    // Validate repository configuration
    await this.validateRepository(request.repository)

    // Auto-detect framework for local repositories if not provided
    let previewConfig = request.previewConfig
    if (!previewConfig?.framework) {
      try {
        const detectedConfig = await this.detectFramework(request.repository.path)
        previewConfig = {
          ...previewConfig,
          ...detectedConfig,
          autoDetected: true,
        }
      } catch (error) {
        logger.warn('Failed to auto-detect framework', { error })
      }
    }

    // Set serverCommand from preview config if available and not already set
    let serverCommand = request.codeExecution.serverCommand
    if (!serverCommand && previewConfig?.serverStartCommand) {
      serverCommand = previewConfig.serverStartCommand
    }

    const repository: RepositoryConfig = {
      id: randomUUID(),
      name: request.name,
      description: request.description,
      repository: request.repository,
      codeExecution: {
        ...request.codeExecution,
        serverCommand,
      },
      previewConfig,
      variables: request.variables || {},
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    await this.saveRepository(repository)
    return repository
  }

  async saveRepository(repo: RepositoryConfig): Promise<void> {
    repo.updatedAt = new Date()
    await this.storage.saveRepository(repo)
  }

  async getRepositoryById(repoId: string): Promise<RepositoryConfig | null> {
    return await this.storage.getRepository(repoId)
  }

  async listRepositories(): Promise<RepositoryConfig[]> {
    const keys = await this.storage.list('repositories/')
    const repositories: RepositoryConfig[] = []

    for (const key of keys) {
      const repoId = key.replace('repositories/', '')
      const repo = await this.storage.getRepository(repoId)
      if (repo) repositories.push(repo)
    }

    // Sort by creation date (newest first)
    return repositories.sort((a, b) => b.createdAt.getTime() - a.createdAt.getTime())
  }

  async updateRepository(
    repoId: string,
    updates: Partial<RepositoryConfig>,
  ): Promise<RepositoryConfig> {
    const repository = await this.getRepositoryById(repoId)
    if (!repository) {
      throw new RepositoryNotFoundError(repoId)
    }

    // Validate repository if being updated
    if (updates.repository) {
      await this.validateRepository(updates.repository)
    }

    const updatedRepository = {
      ...repository,
      ...updates,
      id: repoId, // Ensure ID doesn't change
      updatedAt: new Date(),
    }

    await this.saveRepository(updatedRepository)
    return updatedRepository
  }

  async deleteRepository(repoId: string): Promise<void> {
    const repository = await this.getRepositoryById(repoId)
    if (!repository) {
      throw new RepositoryNotFoundError(repoId)
    }

    // Check if repository is in use by any tasks
    const tasks = await this.storage.list('tasks/')
    for (const taskKey of tasks) {
      const taskId = taskKey.replace('tasks/', '')
      const task = await this.storage.getTask(taskId)
      if (task && task.repositoryId === repoId) {
        throw new ValidationError(
          `Cannot delete repository ${repoId} - it is in use by task ${taskId}`,
          { repositoryId: repoId, taskId },
        )
      }
    }

    await this.storage.delete(`repositories/${repoId}`)
  }

  async runSetupScript(repository: RepositoryConfig): Promise<SetupScriptResponse> {
    if (!repository.codeExecution.setupCommands) {
      return {
        success: true,
        commands: [],
      }
    }

    const commands = repository.codeExecution.setupCommands
      .split('\n')
      .map((cmd: string) => cmd.trim())
      .filter((cmd: string) => cmd.length > 0)

    const results = []
    let allSucceeded = true

    for (const command of commands) {
      try {
        // Execute command in repository path
        const result = await this.executeCommand(command, repository.repository.path)
        results.push(result)
        if (!result.success) {
          allSucceeded = false
        }
      } catch (error) {
        const errorResult = {
          command,
          success: false,
          exit_code: 1,
          error: error instanceof Error ? error.message : String(error),
        }
        results.push(errorResult)
        allSucceeded = false
      }
    }

    return {
      success: allSucceeded,
      commands: results,
    }
  }

  async getBranches(repoPath: string): Promise<string[]> {
    try {
      const result = await this.executeCommand('git branch -a', repoPath)
      if (!result.success || !result.output) {
        return []
      }

      return result.output
        .split('\n')
        .map((line) => line.trim())
        .filter((line) => line.length > 0)
        .map((line) => line.replace(/^\*\s*/, '')) // Remove current branch marker
        .map((line) => line.replace(/^remotes\/origin\//, '')) // Remove remote prefix
        .filter((line) => !line.includes('HEAD ->')) // Remove HEAD pointer
    } catch {
      return []
    }
  }

  async searchFiles(
    repoId: string,
    request: { query: string; limit?: number },
  ): Promise<FileSearchResponse> {
    const repository = await this.getRepositoryById(repoId)
    if (!repository) {
      throw new RepositoryNotFoundError(repoId)
    }

    return await searchFilesInDirectoryNew(repository.repository.path, request)
  }

  async validateRepository(repository: Repository): Promise<void> {
    // Only support local repositories now
    if (repository.type !== 'local') {
      throw new ValidationError('Only local repositories are supported', { type: repository.type })
    }

    await this.validateLocalRepository(repository)
  }

  private async validateLocalRepository(repo: Repository): Promise<void> {
    try {
      const stats = await fs.stat(repo.path)
      if (!stats.isDirectory()) {
        throw new ValidationError(`Path ${repo.path} is not a directory`, { path: repo.path })
      }

      // Check if it's a git repository
      const gitPath = path.join(repo.path, '.git')
      try {
        await fs.access(gitPath)
      } catch {
        throw new ValidationError(`Path ${repo.path} is not a git repository`, { path: repo.path })
      }
    } catch (error) {
      if (error instanceof ValidationError) {
        throw error
      }
      throw new ValidationError(
        `Cannot access repository at ${repo.path}: ${error instanceof Error ? error.message : String(error)}`,
        { path: repo.path, error: error instanceof Error ? error.message : String(error) },
      )
    }
  }

  private async executeCommand(
    command: string,
    cwd: string,
  ): Promise<{
    command: string
    success: boolean
    exit_code: number
    output?: string
    error?: string
  }> {
    const { spawn } = await import('node:child_process')

    return new Promise((resolve) => {
      const [cmd, ...args] = command.split(' ')
      const child = spawn(cmd, args, {
        cwd,
        stdio: ['pipe', 'pipe', 'pipe'],
        shell: true,
      })

      let stdout = ''
      let stderr = ''

      child.stdout?.on('data', (data) => {
        stdout += data.toString()
      })

      child.stderr?.on('data', (data) => {
        stderr += data.toString()
      })

      child.on('close', (code) => {
        resolve({
          command,
          success: code === 0,
          exit_code: code || 0,
          output: stdout.trim() || undefined,
          error: stderr.trim() || undefined,
        })
      })

      child.on('error', (error) => {
        resolve({
          command,
          success: false,
          exit_code: 1,
          error: error.message,
        })
      })
    })
  }

  /**
   * Detect the framework type and version from a local repository path
   */
  private async detectFramework(repoPath: string): Promise<Partial<PreviewConfig>> {
    const packageJsonPath = path.join(repoPath, 'package.json')

    try {
      // Read package.json to detect dependencies
      const packageJson = JSON.parse(await fs.readFile(packageJsonPath, 'utf-8'))
      const deps = { ...packageJson.dependencies, ...packageJson.devDependencies }
      const scripts = packageJson.scripts || {}

      // Detect framework based on dependencies
      if (deps.vite) {
        const serverCommand = this.detectServerCommand(scripts, ['dev', 'start', 'serve'])
        return {
          framework: 'vite',
          frameworkVersion: deps.vite,
          serverStartCommand: serverCommand,
          serverPort: 5173, // Vite default port
          injectionStrategy: {
            method: 'html-injection',
            config: {
              indexPath: 'index.html',
              insertLocation: 'head',
            },
          },
        }
      }

      if (deps.next) {
        const serverCommand = this.detectServerCommand(scripts, ['dev', 'start'])
        return {
          framework: 'nextjs',
          frameworkVersion: deps.next,
          serverStartCommand: serverCommand,
          serverPort: 3000, // Next.js default port
          injectionStrategy: {
            method: 'middleware',
            config: {
              middleware: 'api',
            },
          },
        }
      }

      if (deps['react-scripts']) {
        const serverCommand = this.detectServerCommand(scripts, ['start', 'dev'])
        return {
          framework: 'cra',
          frameworkVersion: deps['react-scripts'],
          serverStartCommand: serverCommand,
          serverPort: 3000, // CRA default port
          injectionStrategy: {
            method: 'html-injection',
            config: {
              indexPath: 'public/index.html',
              insertLocation: 'head',
            },
          },
        }
      }

      if (deps.webpack) {
        const serverCommand = this.detectServerCommand(scripts, ['dev', 'start', 'serve'])
        return {
          framework: 'webpack',
          frameworkVersion: deps.webpack,
          serverStartCommand: serverCommand,
          serverPort: 8080, // Webpack dev server default port
          injectionStrategy: {
            method: 'html-injection',
            config: {},
          },
        }
      }

      // Unknown framework, try to detect start command
      const serverCommand = this.detectServerCommand(scripts, ['dev', 'start', 'serve'])
      return {
        framework: 'unknown',
        serverStartCommand: serverCommand,
        injectionStrategy: {
          method: 'html-injection',
          config: {},
        },
      }
    } catch (error) {
      logger.debug('Failed to detect framework', { repoPath, error })
      return {
        framework: 'unknown',
        injectionStrategy: {
          method: 'html-injection',
          config: {},
        },
      }
    }
  }

  /**
   * Detect the server start command from package.json scripts
   */
  private detectServerCommand(
    scripts: Record<string, string>,
    preferredCommands: string[],
  ): string | undefined {
    // Check preferred commands in order
    for (const cmd of preferredCommands) {
      if (scripts[cmd]) {
        return `npm run ${cmd}`
      }
    }

    // Look for any script containing 'dev' or 'start'
    for (const [scriptName, _scriptValue] of Object.entries(scripts)) {
      if (scriptName.includes('dev') || scriptName.includes('start')) {
        return `npm run ${scriptName}`
      }
    }

    return undefined
  }
}

export function createRepositoryService(
  _app: FastifyInstance,
  storage: StorageBackend,
): RepositoryService {
  return new RepositoryService(storage)
}
