import { promises as fs } from 'node:fs'
import path from 'node:path'
import type {
  Event,
  FigmaImportMetadata,
  RepositoryConfig,
  Task,
  User,
  Workspace,
} from '@moatless/types'
import { fromJSON, removeNullsFromOptionalFields, toJSON } from '@moatless/types'
import { getDataDir, logger } from '../utils/index.js'
import type { StorageBackend } from './storage-backend.js'

export class FileStorage implements StorageBackend {
  constructor(protected basePath: string) {
    logger.info(`Using data directory: ${basePath}`)
  }

  async save(key: string, data: unknown): Promise<void> {
    const filePath = await this.getPath(key)
    const dir = path.dirname(filePath)
    await fs.mkdir(dir, { recursive: true })
    await fs.writeFile(filePath, toJSON(data))
  }

  async get<T = unknown>(key: string): Promise<T | null> {
    const filePath = await this.getPath(key)
    try {
      const content = await fs.readFile(filePath, 'utf-8')
      return fromJSON<T>(content)
    } catch (error) {
      // Log with error details for debugging
      const errorCode = (error as NodeJS.ErrnoException).code
      const errorMessage = error instanceof Error ? error.message : String(error)
      logger.warn(`Failed to read file: ${filePath}`, {
        key,
        errorCode,
        errorMessage,
        error: error instanceof Error ? error : undefined,
      })
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
    // Clean null values from optional fields before saving
    const taskOptionalFields = ['workspacePath']
    const cleanedTask = removeNullsFromOptionalFields(task, taskOptionalFields)
    await this.save(`tasks/${cleanedTask.taskId}`, cleanedTask)
  }

  async getTask(taskId: string): Promise<Task | null> {
    return await this.get<Task>(`tasks/${taskId}`)
  }

  async saveRepository(repo: RepositoryConfig): Promise<void> {
    await this.save(`repositories/${repo.id}`, repo)
  }

  async getRepository(repoId: string): Promise<RepositoryConfig | null> {
    return await this.get<RepositoryConfig>(`repositories/${repoId}`)
  }

  async saveWorkspace(workspace: Workspace): Promise<void> {
    await this.save(`workspaces/${workspace.id}`, workspace)
  }

  async getWorkspace(taskId: string): Promise<Workspace | null> {
    return await this.get<Workspace>(`workspaces/${taskId}`)
  }

  async saveEvents(taskId: string, events: Event[]): Promise<void> {
    await this.save(`events/${taskId}`, events)
  }

  async getEvents(taskId: string): Promise<Event[]> {
    const events = await this.get<Event[]>(`events/${taskId}`)
    return events || []
  }

  async saveUser(user: User): Promise<void> {
    await this.save(`users/${user.id}`, user)
  }

  async getUser(userId: string): Promise<User | null> {
    return await this.get<User>(`users/${userId}`)
  }

  async listUsers(): Promise<User[]> {
    const userKeys = await this.list('users/')
    const users: User[] = []

    for (const key of userKeys) {
      const user = await this.get<User>(key)
      if (user) {
        users.push(user)
      }
    }

    return users
  }

  async saveFigmaImport(metadata: FigmaImportMetadata): Promise<void> {
    await this.save(`figma-imports/${metadata.importId}`, metadata)
  }

  async getFigmaImport(importId: string): Promise<FigmaImportMetadata | null> {
    return await this.get<FigmaImportMetadata>(`figma-imports/${importId}`)
  }

  // JSONL specific methods for message persistence
  async appendJsonl(key: string, data: unknown): Promise<void> {
    const filePath = await this.getPath(key, '.jsonl')
    const dir = path.dirname(filePath)
    await fs.mkdir(dir, { recursive: true })
    await fs.appendFile(filePath, `${toJSON(data)}\n`, 'utf-8')
  }

  async readJsonl<T = unknown>(key: string): Promise<T[]> {
    const filePath = await this.getPath(key, '.jsonl')
    const results: T[] = []

    try {
      const content = await fs.readFile(filePath, 'utf-8')
      const lines = content.trim().split('\n').filter(Boolean)

      for (const line of lines) {
        try {
          const parsed = fromJSON<T>(line)
          results.push(parsed)
        } catch (parseError) {
          logger.warn(`Failed to parse JSONL line in file: ${filePath}`, {
            error: parseError instanceof Error ? parseError.message : String(parseError),
          })
          // Continue processing other lines
        }
      }
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') {
        logger.error(`Failed to read JSONL file: ${filePath}`, error as Error)
      }
      // Return empty array for missing files
    }

    return results
  }

  protected getPath(key: string, extension = '.json'): string {
    return path.join(this.basePath, `${key}${extension}`)
  }
}

export async function createStorageService(): Promise<StorageBackend> {
  const dataDir = getDataDir()

  // Ensure the data directory exists
  await fs.mkdir(dataDir, { recursive: true })

  return new FileStorage(dataDir)
}
