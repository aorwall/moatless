import type { FigmaImportMetadata, Task, Workspace } from '@moatless/types'
import type { ServiceIdentifier } from 'inversify'

/**
 * DI symbol for StorageBackend interface.
 * Use this symbol to inject storage implementations.
 */
export const StorageSymbol: ServiceIdentifier<StorageBackend> = Symbol.for('Storage')

export interface StorageBackend {
  // Generic storage methods
  save(key: string, data: unknown): Promise<void>
  get<T = unknown>(key: string): Promise<T | null>
  list(prefix?: string): Promise<string[]>
  delete(key: string): Promise<void>
  exists(key: string): Promise<boolean>

  // JSONL methods for append-only data
  appendJsonl(key: string, data: unknown): Promise<void>
  readJsonl<T = unknown>(key: string): Promise<T[]>

  // Type-safe convenience methods
  saveTask(task: Task): Promise<void>
  getTask(taskId: string): Promise<Task | null>
  saveWorkspace(workspace: Workspace): Promise<void>
  getWorkspace(taskId: string): Promise<Workspace | null>
  saveFigmaImport(metadata: FigmaImportMetadata): Promise<void>
  getFigmaImport(importId: string): Promise<FigmaImportMetadata | null>
}
