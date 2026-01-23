import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  bumpSnapshotRevision,
  type WorkspaceRuntimeSnapshot,
} from '../../types/workspace-runtime.types'
import { WorkspaceInactivityService } from '../workspace-inactivity.service'

class InMemoryRuntimeStore {
  private readonly snapshots = new Map<string, WorkspaceRuntimeSnapshot>()

  async load(taskId: string): Promise<WorkspaceRuntimeSnapshot | null> {
    return this.snapshots.get(taskId) ?? null
  }

  async save(snapshot: WorkspaceRuntimeSnapshot): Promise<void> {
    this.snapshots.set(snapshot.taskId, snapshot)
  }

  async update(
    taskId: string,
    updater: (snapshot: WorkspaceRuntimeSnapshot) => WorkspaceRuntimeSnapshot,
  ): Promise<boolean> {
    const current = this.snapshots.get(taskId)
    if (!current) {
      return false
    }
    const updated = bumpSnapshotRevision(updater(current))
    this.snapshots.set(taskId, updated)
    return true
  }

  async list(): Promise<WorkspaceRuntimeSnapshot[]> {
    return [...this.snapshots.values()]
  }

  async touchActivity(taskId: string, timestamp: Date): Promise<void> {
    const snapshot = this.snapshots.get(taskId)
    if (snapshot) {
      this.snapshots.set(taskId, { ...snapshot, lastActivityAt: timestamp, updatedAt: timestamp })
    }
  }

  // Helper for tests
  set(snapshot: WorkspaceRuntimeSnapshot): void {
    this.snapshots.set(snapshot.taskId, snapshot)
  }

  get(taskId: string): WorkspaceRuntimeSnapshot | undefined {
    return this.snapshots.get(taskId)
  }
}

function createSnapshot(
  overrides: Partial<WorkspaceRuntimeSnapshot> = {},
): WorkspaceRuntimeSnapshot {
  const now = new Date()
  return {
    taskId: 'task-1',
    repositoryId: 'repo-1',
    snapshotRevision: '1',
    executionTarget: {
      mode: 'kubernetes',
      podName: 'pod-task-1',
      podNamespace: 'default',
      repositoryCloneUrl: 'https://github.com/test/repo.git',
      workerPort: 8001,
      status: 'ready',
    },
    preview: { status: 'stopped' },
    createdAt: now,
    updatedAt: now,
    ...overrides,
  }
}

describe('WorkspaceInactivityService', () => {
  let service: WorkspaceInactivityService
  let store: InMemoryRuntimeStore
  let runtimeManager: { stopAgentAndInvalidate: ReturnType<typeof vi.fn> }
  let provisioner: {
    clearError: ReturnType<typeof vi.fn>
    cleanup: ReturnType<typeof vi.fn>
  }

  beforeEach(() => {
    vi.stubEnv('WORKSPACE_INACTIVITY_TIMEOUT_MINUTES', '20')

    store = new InMemoryRuntimeStore()
    runtimeManager = {
      stopAgentAndInvalidate: vi.fn().mockResolvedValue(undefined),
    }
    provisioner = {
      clearError: vi.fn().mockResolvedValue(undefined),
      cleanup: vi.fn().mockResolvedValue(undefined),
    }

    service = new WorkspaceInactivityService(
      runtimeManager as any,
      store as any,
      provisioner as any,
    )
  })

  afterEach(() => {
    service.stop()
    vi.unstubAllEnvs()
  })

  it('suspends inactive workspace', async () => {
    const thirtyMinAgo = new Date(Date.now() - 30 * 60 * 1000)
    store.set(
      createSnapshot({
        lastActivityAt: thirtyMinAgo,
        updatedAt: thirtyMinAgo,
      }),
    )

    await service.checkInactiveWorkspaces()

    const snapshot = store.get('task-1')
    expect(snapshot?.executionTarget.status).toBe('stopped')
    expect(provisioner.clearError).toHaveBeenCalledWith(
      'task-1',
      expect.objectContaining({ mode: 'kubernetes' }),
    )
  })

  it('does not suspend active workspace', async () => {
    const fiveMinAgo = new Date(Date.now() - 5 * 60 * 1000)
    store.set(
      createSnapshot({
        lastActivityAt: fiveMinAgo,
        updatedAt: fiveMinAgo,
      }),
    )

    await service.checkInactiveWorkspaces()

    const snapshot = store.get('task-1')
    expect(snapshot?.executionTarget.status).toBe('ready')
    expect(provisioner.clearError).not.toHaveBeenCalled()
  })

  it('does not suspend workspace with running agent', async () => {
    const thirtyMinAgo = new Date(Date.now() - 30 * 60 * 1000)
    store.set(
      createSnapshot({
        lastActivityAt: thirtyMinAgo,
        updatedAt: thirtyMinAgo,
        agent: { state: 'running', sessionId: 'session-1' },
      }),
    )

    await service.checkInactiveWorkspaces()

    const snapshot = store.get('task-1')
    expect(snapshot?.executionTarget.status).toBe('ready')
    expect(provisioner.clearError).not.toHaveBeenCalled()
  })

  it('skips non-ready workspace', async () => {
    const thirtyMinAgo = new Date(Date.now() - 30 * 60 * 1000)
    store.set(
      createSnapshot({
        lastActivityAt: thirtyMinAgo,
        updatedAt: thirtyMinAgo,
        executionTarget: {
          mode: 'kubernetes',
          podName: 'pod-task-1',
          podNamespace: 'default',
          repositoryCloneUrl: 'https://github.com/test/repo.git',
          workerPort: 8001,
          status: 'stopped',
        },
      }),
    )

    await service.checkInactiveWorkspaces()

    expect(provisioner.clearError).not.toHaveBeenCalled()
  })

  it('does nothing when timeout is 0 (disabled)', async () => {
    vi.stubEnv('WORKSPACE_INACTIVITY_TIMEOUT_MINUTES', '0')

    const thirtyMinAgo = new Date(Date.now() - 30 * 60 * 1000)
    store.set(
      createSnapshot({
        lastActivityAt: thirtyMinAgo,
        updatedAt: thirtyMinAgo,
      }),
    )

    await service.checkInactiveWorkspaces()

    const snapshot = store.get('task-1')
    expect(snapshot?.executionTarget.status).toBe('ready')
    expect(provisioner.clearError).not.toHaveBeenCalled()
  })

  it('errors in one workspace do not prevent checking others', async () => {
    const thirtyMinAgo = new Date(Date.now() - 30 * 60 * 1000)

    // First workspace will throw during clearError
    store.set(
      createSnapshot({
        taskId: 'task-fail',
        lastActivityAt: thirtyMinAgo,
        updatedAt: thirtyMinAgo,
      }),
    )
    provisioner.clearError.mockRejectedValueOnce(new Error('pod not found'))

    // Second workspace should still be processed
    store.set(
      createSnapshot({
        taskId: 'task-ok',
        lastActivityAt: thirtyMinAgo,
        updatedAt: thirtyMinAgo,
      }),
    )

    await service.checkInactiveWorkspaces()

    // task-fail still gets status update despite clearError failure
    const failSnapshot = store.get('task-fail')
    expect(failSnapshot?.executionTarget.status).toBe('stopped')

    // task-ok is also suspended
    const okSnapshot = store.get('task-ok')
    expect(okSnapshot?.executionTarget.status).toBe('stopped')
  })

  it('updates snapshot status to stopped after suspend', async () => {
    const thirtyMinAgo = new Date(Date.now() - 30 * 60 * 1000)
    store.set(
      createSnapshot({
        lastActivityAt: thirtyMinAgo,
        updatedAt: thirtyMinAgo,
      }),
    )

    await service.checkInactiveWorkspaces()

    const snapshot = store.get('task-1')
    expect(snapshot?.executionTarget.status).toBe('stopped')
    // Snapshot is preserved (not deleted)
    expect(snapshot?.taskId).toBe('task-1')
    expect(snapshot?.repositoryId).toBe('repo-1')
  })

  it('uses clearError for Kubernetes (pod-only deletion)', async () => {
    const thirtyMinAgo = new Date(Date.now() - 30 * 60 * 1000)
    store.set(
      createSnapshot({
        lastActivityAt: thirtyMinAgo,
        updatedAt: thirtyMinAgo,
        executionTarget: {
          mode: 'kubernetes',
          podName: 'pod-task-1',
          podNamespace: 'default',
          repositoryCloneUrl: 'https://github.com/test/repo.git',
          workerPort: 8001,
          status: 'ready',
        },
      }),
    )

    await service.checkInactiveWorkspaces()

    expect(provisioner.clearError).toHaveBeenCalledWith(
      'task-1',
      expect.objectContaining({ mode: 'kubernetes' }),
    )
    expect(provisioner.cleanup).not.toHaveBeenCalled()
  })

  it('uses cleanup for Docker (container removal)', async () => {
    const thirtyMinAgo = new Date(Date.now() - 30 * 60 * 1000)
    store.set(
      createSnapshot({
        lastActivityAt: thirtyMinAgo,
        updatedAt: thirtyMinAgo,
        executionTarget: {
          mode: 'docker',
          containerName: 'container-task-1',
          repositoryCloneUrl: 'https://github.com/test/repo.git',
          status: 'ready',
        },
      }),
    )

    await service.checkInactiveWorkspaces()

    expect(provisioner.cleanup).toHaveBeenCalledWith(
      'task-1',
      expect.objectContaining({ mode: 'docker' }),
    )
    expect(provisioner.clearError).not.toHaveBeenCalled()
  })

  it('falls back to updatedAt when lastActivityAt is not set', async () => {
    const thirtyMinAgo = new Date(Date.now() - 30 * 60 * 1000)
    store.set(
      createSnapshot({
        lastActivityAt: undefined,
        updatedAt: thirtyMinAgo,
      }),
    )

    await service.checkInactiveWorkspaces()

    const snapshot = store.get('task-1')
    expect(snapshot?.executionTarget.status).toBe('stopped')
  })

  it('calls stopAgentAndInvalidate before stopping resources', async () => {
    const thirtyMinAgo = new Date(Date.now() - 30 * 60 * 1000)
    store.set(
      createSnapshot({
        lastActivityAt: thirtyMinAgo,
        updatedAt: thirtyMinAgo,
      }),
    )

    const callOrder: string[] = []
    runtimeManager.stopAgentAndInvalidate.mockImplementation(async () => {
      callOrder.push('stopAgent')
    })
    provisioner.clearError.mockImplementation(async () => {
      callOrder.push('clearError')
    })

    await service.checkInactiveWorkspaces()

    expect(callOrder).toEqual(['stopAgent', 'clearError'])
  })

  it('start does not set interval when timeout is 0', () => {
    vi.stubEnv('WORKSPACE_INACTIVITY_TIMEOUT_MINUTES', '0')

    const setIntervalSpy = vi.spyOn(globalThis, 'setInterval')
    service.start()

    expect(setIntervalSpy).not.toHaveBeenCalled()
    setIntervalSpy.mockRestore()
  })
})
