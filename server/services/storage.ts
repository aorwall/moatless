import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { logger } from '@server/utils/index.js'
import type { FastifyInstance } from 'fastify'
import type { Environment, Event, Task, Workspace } from '../../shared/types/index.js'
import type { RepositoryConfig } from '../../shared/types/repository.js'
import { fromJSON, toJSON } from '../../shared/utils/validation.js'

export interface StorageBackend {
  // Generic storage methods
  save(key: string, data: unknown): Promise<void>
  get<T = unknown>(key: string): Promise<T | null>
  list(prefix?: string): Promise<string[]>
  delete(key: string): Promise<void>
  exists(key: string): Promise<boolean>

  // Type-safe convenience methods
  saveTask(task: Task): Promise<void>
  getTask(taskId: string): Promise<Task | null>
  saveEnvironment(env: Environment): Promise<void>
  getEnvironment(envId: string): Promise<Environment | null>
  saveRepository(repo: RepositoryConfig): Promise<void>
  getRepository(repoId: string): Promise<RepositoryConfig | null>
  saveWorkspace(workspace: Workspace): Promise<void>
  getWorkspace(taskId: string): Promise<Workspace | null>
  saveEvents(taskId: string, events: Event[]): Promise<void>
  getEvents(taskId: string): Promise<Event[]>
}

export class FileStorage implements StorageBackend {
  constructor(private basePath: string) {
    logger.info(`Using data directory: ${basePath}`)
  }

  async save(key: string, data: unknown): Promise<void> {
    const filePath = await this.getPath(key)
    const dir = path.dirname(filePath)
    await fs.mkdir(dir, { recursive: true })
    await fs.writeFile(filePath, toJSON(data))
  }

  async get<T = unknown>(key: string): Promise<T | null> {
    try {
      const filePath = await this.getPath(key)
      const content = await fs.readFile(filePath, 'utf-8')
      return fromJSON<T>(content)
    } catch {
      return null
    }
  }

  async list(prefix?: string): Promise<string[]> {
    const dir = path.join(this.basePath, prefix || '')
    try {
      const entries = await fs.readdir(dir, { withFileTypes: true })
      const files: string[] = []

      for (const entry of entries) {
        if (entry.isFile() && entry.name.endsWith('.json')) {
          const key = prefix
            ? `${prefix}${entry.name.replace('.json', '')}`
            : entry.name.replace('.json', '')
          files.push(key)
        } else if (entry.isDirectory()) {
          // Recursively list subdirectories
          const subPrefix = prefix ? `${prefix}${entry.name}/` : `${entry.name}/`
          const subFiles = await this.list(subPrefix)
          files.push(...subFiles)
        }
      }

      return files.sort()
    } catch {
      return []
    }
  }

  async delete(key: string): Promise<void> {
    const filePath = await this.getPath(key)
    try {
      await fs.unlink(filePath)
    } catch (error) {
      // Ignore ENOENT errors (file not found) - if the file doesn't exist,
      // the delete operation has already achieved its goal
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') {
        throw error
      }
    }
  }

  async exists(key: string): Promise<boolean> {
    const filePath = await this.getPath(key)
    try {
      await fs.access(filePath)
      return true
    } catch {
      return false
    }
  }

  // Type-safe convenience methods
  async saveTask(task: Task): Promise<void> {
    await this.save(`tasks/${task.taskId}`, task)
  }

  async getTask(taskId: string): Promise<Task | null> {
    return this.get<Task>(`tasks/${taskId}`)
  }

  async saveEnvironment(env: Environment): Promise<void> {
    await this.save(`environments/${env.id}`, env)
  }

  async getEnvironment(envId: string): Promise<Environment | null> {
    return this.get<Environment>(`environments/${envId}`)
  }

  async saveRepository(repo: RepositoryConfig): Promise<void> {
    await this.save(`repositories/${repo.id}`, repo)
  }

  async getRepository(repoId: string): Promise<RepositoryConfig | null> {
    return this.get<RepositoryConfig>(`repositories/${repoId}`)
  }

  async saveWorkspace(workspace: Workspace): Promise<void> {
    await this.save(`workspaces/${workspace.id}`, workspace)
  }

  async getWorkspace(taskId: string): Promise<Workspace | null> {
    return this.get<Workspace>(`workspaces/${taskId}`)
  }

  async saveEvents(taskId: string, events: Event[]): Promise<void> {
    await this.save(`events/${taskId}`, events)
  }

  async getEvents(taskId: string): Promise<Event[]> {
    const events = await this.get<Event[]>(`events/${taskId}`)
    return events || []
  }

  private async getPath(key: string): Promise<string> {
    return path.join(this.basePath, `${key}.json`)
  }
}

function getDefaultDataDir(): string {
  return path.join(os.homedir(), '.moatless')
}

export async function createStorageService(_app: FastifyInstance): Promise<StorageBackend> {
  const dataDir = process.env.DATA_DIR || getDefaultDataDir()

  // Ensure the data directory exists
  await fs.mkdir(dataDir, { recursive: true })

  return new FileStorage(dataDir)
}
