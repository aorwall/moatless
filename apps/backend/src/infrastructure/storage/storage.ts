/**
 * Minimal storage interface for feature-scoped persistence adapters.
 *
 * New feature modules should depend on this interface instead of the legacy
 * StorageBackend located under `apps/backend/src/storage`.
 */
export interface Storage {
  /** Persist a JSON-serializable payload under the provided key. */
  save(key: string, data: unknown): Promise<void>

  /** Retrieve data for the given key, returning null when it does not exist. */
  get<T = unknown>(key: string): Promise<T | null>

  /** Remove the data associated with the supplied key. */
  delete(key: string): Promise<void>

  /** Determine whether data exists for the supplied key. */
  exists(key: string): Promise<boolean>
}
