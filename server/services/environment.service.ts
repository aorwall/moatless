import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { EnvironmentNotFoundError, ValidationError } from '@shared/constants/errors.js'
import type { FastifyInstance } from 'fastify'
import type {
  CreateEnvironmentRequest,
  Environment,
  FileSearchRequest,
  FileSearchResponse,
  LocalRepository,
  PreviewConfig,
  RemoteRepository,
} from '../../shared/types/index.js'
import { searchFilesInDirectoryNew } from '../utils/file-search.js'
import { logger } from '../utils/logger.js'
import { validateRepositoryAccess } from '../utils/repository.js'
import type { StorageBackend } from './storage.js'

export class EnvironmentService {
  constructor(private storage: StorageBackend) {}

  async createEnvironment(request: CreateEnvironmentRequest): Promise<Environment> {
    // Validate repository configuration
    await this.validateRepository(request.repository)

    // Auto-detect framework for local repositories if not provided
    let previewConfig = request.previewConfig
    if (request.repository.type === 'local' && !previewConfig?.framework) {
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

    const environment: Environment = {
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

    await this.saveEnvironment(environment)
    return environment
  }

  async saveEnvironment(env: Environment): Promise<void> {
    env.updatedAt = new Date()
    await this.storage.saveEnvironment(env)
  }

  async getEnvironment(envId: string): Promise<Environment | null> {
    return await this.storage.getEnvironment(envId)
  }

  async listEnvironments(): Promise<Environment[]> {
    const keys = await this.storage.list('environments/')
    const environments: Environment[] = []

    for (const key of keys) {
      const envId = key.replace('environments/', '')
      const env = await this.storage.getEnvironment(envId)
      if (env) environments.push(env)
    }

    // Sort by creation date (newest first)
    return environments.sort((a, b) => b.createdAt.getTime() - a.createdAt.getTime())
  }

  async updateEnvironment(envId: string, updates: Partial<Environment>): Promise<Environment> {
    const environment = await this.getEnvironment(envId)
    if (!environment) {
      throw new EnvironmentNotFoundError(envId)
    }

    // Validate repository if being updated
    if (updates.repository) {
      await this.validateRepository(updates.repository)
    }

    const updatedEnvironment = {
      ...environment,
      ...updates,
      id: envId, // Ensure ID doesn't change
      updatedAt: new Date(),
    }

    await this.saveEnvironment(updatedEnvironment)
    return updatedEnvironment
  }

  async deleteEnvironment(envId: string): Promise<void> {
    const environment = await this.getEnvironment(envId)
    if (!environment) {
      throw new EnvironmentNotFoundError(envId)
    }

    // Check if environment is in use by any tasks
    const tasks = await this.storage.list('tasks/')
    for (const taskKey of tasks) {
      const taskId = taskKey.replace('tasks/', '')
      const task = await this.storage.getTask(taskId)
      if (task && task.environmentId === envId) {
        throw new ValidationError(
          `Cannot delete environment ${envId} - it is in use by task ${taskId}`,
          { environmentId: envId, taskId },
        )
      }
    }

    await this.storage.delete(`environments/${envId}`)
  }

  async searchFiles(request: FileSearchRequest): Promise<FileSearchResponse> {
    const environment = await this.getEnvironment(request.environmentId)
    if (!environment) {
      throw new EnvironmentNotFoundError(request.environmentId)
    }

    // Only support file search for local repositories currently
    if (environment.repository.type !== 'local') {
      throw new ValidationError('File search is only supported for local repositories', {
        repositoryType: environment.repository.type,
      })
    }

    const localRepo = environment.repository as LocalRepository

    return await searchFilesInDirectoryNew(localRepo.path, request)
  }

  async validateEnvironment(envId: string): Promise<boolean> {
    const environment = await this.getEnvironment(envId)
    if (!environment) {
      return false
    }

    try {
      await this.validateRepository(environment.repository)
      return true
    } catch {
      return false
    }
  }

  /**
   * Re-detect framework for an environment and update its configuration
   */
  async detectAndUpdateFramework(envId: string): Promise<Environment> {
    const environment = await this.getEnvironment(envId)
    if (!environment) {
      throw new EnvironmentNotFoundError(envId)
    }

    if (environment.repository.type !== 'local') {
      throw new ValidationError('Framework detection is only supported for local repositories', {
        repositoryType: environment.repository.type,
      })
    }

    const localRepo = environment.repository as LocalRepository
    const detectedConfig = await this.detectFramework(localRepo.path)

    const updatedEnvironment = {
      ...environment,
      previewConfig: {
        ...detectedConfig,
        autoDetected: true,
      },
      updatedAt: new Date(),
    }

    await this.saveEnvironment(updatedEnvironment)
    return updatedEnvironment
  }

  async getRepositoryInfo(envId: string): Promise<{
    type: string
    accessible: boolean
    details: Record<string, unknown>
  }> {
    const environment = await this.getEnvironment(envId)
    if (!environment) {
      throw new EnvironmentNotFoundError(envId)
    }

    const repository = environment.repository

    try {
      const details = await this.getRepositoryDetails(repository)
      return {
        type: repository.type,
        accessible: true,
        details,
      }
    } catch (error) {
      return {
        type: repository.type,
        accessible: false,
        details: { error: error instanceof Error ? error.message : String(error) },
      }
    }
  }

  private async validateRepository(repository: Repository): Promise<void> {
    switch (repository.type) {
      case 'local':
        await this.validateLocalRepository(repository as LocalRepository)
        break
      case 'github':
      case 'gitlab':
        await this.validateRemoteRepository(repository as RemoteRepository)
        break
      default:
        throw new ValidationError(`Unsupported repository type: ${repository.type}`, {
          repositoryType: repository.type,
        })
    }
  }

  private async validateLocalRepository(repo: LocalRepository): Promise<void> {
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

  private async validateRemoteRepository(repo: RemoteRepository): Promise<void> {
    // Validate URL format
    try {
      new URL(repo.url!)
    } catch {
      throw new ValidationError(`Invalid repository URL: ${repo.url}`, { url: repo.url })
    }

    // Additional validation could include checking if repo is accessible
    // This would require implementing git operations or API calls
    await validateRepositoryAccess(repo)
  }

  private async getRepositoryDetails(repository: Repository): Promise<Record<string, unknown>> {
    switch (repository.type) {
      case 'local': {
        const localRepo = repository as LocalRepository
        const stats = await fs.stat(localRepo.path)
        return {
          path: localRepo.path,
          size: stats.size,
          modified: stats.mtime,
          // Could add git info like current branch, commit hash, etc.
        }
      }

      default:
        return {}
    }
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

export function createEnvironmentService(
  _app: FastifyInstance,
  storage: StorageBackend,
): EnvironmentService {
  return new EnvironmentService(storage)
}
