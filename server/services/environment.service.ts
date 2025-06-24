import { EnvironmentNotFoundError, ValidationError } from '@shared/constants/errors.js'
import type {
  CreateEnvironmentRequest,
  Environment,
  FileSearchRequest,
  FileSearchResponse,
  LocalRepository,
  RemoteRepository,
} from '../../shared/types/index.js'
import { randomUUID } from 'crypto'
import type { FastifyInstance } from 'fastify'
import { promises as fs } from 'fs'
import path from 'path'
import { searchFilesInDirectory } from '../utils/file-search.js'
import { validateRepositoryAccess } from '../utils/repository.js'
import type { StorageBackend } from './storage.js'

export class EnvironmentService {
  constructor(private storage: StorageBackend) {}

  async createEnvironment(request: CreateEnvironmentRequest): Promise<Environment> {
    // Validate repository configuration
    await this.validateRepository(request.repository)

    const environment: Environment = {
      id: randomUUID(),
      name: request.name,
      description: request.description,
      repository: request.repository,
      codeExecution: request.codeExecution,
      variables: request.variables || {},
      isActive: true,
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

    return await searchFilesInDirectory(
      localRepo.path,
      request.query,
      request.maxResults || 100,
      request.filePattern,
    )
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

  async getRepositoryInfo(envId: string): Promise<{
    type: string
    accessible: boolean
    details: any
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

  private async validateRepository(repository: any): Promise<void> {
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

  private async getRepositoryDetails(repository: any): Promise<any> {
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
      case 'github':
      case 'gitlab': {
        const remoteRepo = repository as RemoteRepository
        return {
          url: remoteRepo.url,
          branch: remoteRepo.branch,
          commit: remoteRepo.commit,
          // Could add API info like last commit date, repository stats, etc.
        }
      }
      default:
        return {}
    }
  }
}

export function createEnvironmentService(
  _app: FastifyInstance,
  storage: StorageBackend,
): EnvironmentService {
  return new EnvironmentService(storage)
}
