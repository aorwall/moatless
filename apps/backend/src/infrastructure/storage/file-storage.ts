import { promises as fs } from 'node:fs'
import path from 'node:path'
import { fromJSON, toJSON } from '@moatless/types'
import { getDataDir } from '../../utils/data-dir.js'
import { logger } from '../../utils/logger.js'
import type { Storage } from './storage.js'

const JSON_EXTENSION = '.json'

/**
 * Lightweight file-system backed implementation of the new storage interface.
 * Keys are mapped to files relative to the configured base directory using a
 * `.json` extension. Nested keys are supported via `/` separation.
 */
export class FileStorage implements Storage {
  constructor(private readonly baseDir: string = getDataDir()) {
    logger.debug('Initialized infrastructure FileStorage', { baseDir })
  }

  async save(key: string, data: unknown): Promise<void> {
    const filePath = this.resolvePath(key)
    await fs.mkdir(path.dirname(filePath), { recursive: true })

    const tempPath = `${filePath}.tmp.${Date.now()}.${Math.random().toString(36).slice(2)}`
    try {
      await fs.writeFile(tempPath, toJSON(data))
      await fs.rename(tempPath, filePath)
    } catch (error) {
      try {
        await fs.unlink(tempPath)
      } catch {
        // ignore cleanup failures
      }
      throw error
    }
  }

  async get<T = unknown>(key: string): Promise<T | null> {
    const filePath = this.resolvePath(key)
    try {
      const content = await fs.readFile(filePath, 'utf-8')
      return fromJSON<T>(content)
    } catch (error) {
      const err = error as NodeJS.ErrnoException
      if (err.code !== 'ENOENT') {
        logger.warn('Failed to read storage file', { key, filePath, error: err })
      }
      return null
    }
  }

  async delete(key: string): Promise<void> {
    const filePath = this.resolvePath(key)
    try {
      await fs.unlink(filePath)
    } catch (error) {
      const err = error as NodeJS.ErrnoException
      if (err.code !== 'ENOENT') {
        throw err
      }
    }
  }

  async list(prefix?: string): Promise<string[]> {
    const dir = path.join(this.baseDir, prefix || '')
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

  async exists(key: string): Promise<boolean> {
    const filePath = this.resolvePath(key)
    try {
      await fs.access(filePath)
      return true
    } catch {
      return false
    }
  }

  private resolvePath(key: string, extension = JSON_EXTENSION): string {
    const sanitized = key.replace(/^\/+/, '')
    return path.join(this.baseDir, `${sanitized}${extension}`)
  }
}
