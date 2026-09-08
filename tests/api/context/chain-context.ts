/**
 * Typed state accumulator for serial test chains.
 *
 * Each chain spec creates a ChainContext at describe scope and passes it
 * through sequential tests. Steps write to the context; later tests read.
 *
 * Usage:
 *   test.describe("Chain: Task CRUD", () => {
 *     test.describe.configure({ mode: "serial" });
 *     const ctx = new ChainContext();
 *     // ...tests populate and read ctx fields
 *   });
 */

import type {
  RepositoryResponse,
  SandboxStatusFullResponse,
  TaskResponse,
} from '../../shared/types'

export class ChainContext {
  /** Template repository used to create tasks */
  repositoryId?: string
  repository?: RepositoryResponse

  /** Workspace placing the template repository — what a task or loop names */
  workspaceId?: string

  /** Task created at the start of the chain */
  taskId?: string
  task?: TaskResponse

  /** Sandbox/workspace status after provisioning */
  sandboxStatus?: SandboxStatusFullResponse

  /** Preview server URL once started */
  serverUrl?: string

  /** Tracks the number of agent turns completed */
  turnCount = 0

  /** Tracks cost to verify agent work */
  lastCostUsd = 0

  /** Generic key-value store for chain-specific state */
  private extra: Record<string, unknown> = {}

  get<T>(key: string): T | undefined {
    return this.extra[key] as T | undefined
  }

  set(key: string, value: unknown): void {
    this.extra[key] = value
  }
}
