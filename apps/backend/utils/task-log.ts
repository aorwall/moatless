import { promises as fs } from 'node:fs'
import path from 'node:path'
import { getDataDir } from './data-dir.js'

export function getTaskLogPath(taskId: string): string {
  const dataDir = getDataDir()
  return path.join(dataDir, 'logs', `${taskId}.log`)
}

export async function ensureTaskLogDirectory(): Promise<void> {
  const dataDir = getDataDir()
  const logsDir = path.join(dataDir, 'logs')
  await fs.mkdir(logsDir, { recursive: true })
}

export function formatLogEntry(prefix: string, message: string): string {
  const timestamp = new Date().toISOString()
  return `[${timestamp}] [${prefix}] ${message}\n`
}

export async function appendToTaskLog(
  taskId: string,
  prefix: string,
  message: string,
): Promise<void> {
  const logPath = getTaskLogPath(taskId)
  await ensureTaskLogDirectory()
  const entry = formatLogEntry(prefix, message)
  await fs.appendFile(logPath, entry, 'utf-8')
}

export async function readTaskLog(taskId: string, lines?: number): Promise<string> {
  const logPath = getTaskLogPath(taskId)

  try {
    const content = await fs.readFile(logPath, 'utf-8')

    if (!lines) {
      return content
    }

    const allLines = content.split('\n')
    const startIndex = Math.max(0, allLines.length - lines)
    return allLines.slice(startIndex).join('\n')
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
      return ''
    }
    throw error
  }
}

export async function taskLogExists(taskId: string): Promise<boolean> {
  const logPath = getTaskLogPath(taskId)
  try {
    await fs.access(logPath)
    return true
  } catch {
    return false
  }
}

export async function clearTaskLog(taskId: string): Promise<void> {
  const logPath = getTaskLogPath(taskId)
  await ensureTaskLogDirectory()

  try {
    // Truncate the file (or create empty file if it doesn't exist)
    await fs.writeFile(logPath, '', 'utf-8')
  } catch (error) {
    // Log but don't throw - clearing logs shouldn't fail workspace creation
    console.error(`Failed to clear task log for ${taskId}:`, error)
  }
}
