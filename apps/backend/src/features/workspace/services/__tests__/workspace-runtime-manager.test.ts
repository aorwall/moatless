import type { RepositoryConfig } from '@moatless/types'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { WorkspaceProvisionPlan } from '../../types/provision-plan.types'
import type { WorkspaceProvisioner } from '../../types/workspace-provisioner.types'
import {
  bumpSnapshotRevision,
  createWorkspaceSnapshotFromTarget,
  type ExecutionTarget,
  type WorkspaceLock,
  type WorkspaceRuntimeSnapshot,
} from '../../types/workspace-runtime.types'
import { WorkspaceRuntimeManager } from '../workspace-runtime-manager.service'

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

  async delete(taskId: string): Promise<void> {
    this.snapshots.delete(taskId)
  }

  async list(): Promise<WorkspaceRuntimeSnapshot[]> {
    return [...this.snapshots.values()]
  }

  async tryInsert(snapshot: WorkspaceRuntimeSnapshot): Promise<boolean> {
    if (this.snapshots.has(snapshot.taskId)) {
      return false
    }
    this.snapshots.set(snapshot.taskId, snapshot)
    return true
  }

  async tryAcquireLock(taskId: string, lock: WorkspaceLock, ttlMs: number): Promise<boolean> {
    const snapshot = this.snapshots.get(taskId)
    if (!snapshot) {
      return false
    }

    const now = Date.now()
    const activeLocks = (snapshot.locks ?? []).filter(
      (existingLock) => now - existingLock.acquiredAt.getTime() < ttlMs,
    )

    if (activeLocks.some((existingLock) => existingLock.type === lock.type)) {
      return false
    }

    const updated = bumpSnapshotRevision({
      ...snapshot,
      locks: [...activeLocks, lock],
    })
    this.snapshots.set(taskId, updated)
    return true
  }

  async releaseLock(taskId: string, type: WorkspaceLock['type'], owner: string): Promise<void> {
    const snapshot = this.snapshots.get(taskId)
    if (!snapshot) {
      return
    }

    const locks = (snapshot.locks ?? []).filter(
      (lock) => !(lock.type === type && lock.owner === owner),
    )
    this.snapshots.set(
      taskId,
      bumpSnapshotRevision({
        ...snapshot,
        locks: locks.length > 0 ? locks : undefined,
      }),
    )
  }
}

const makePlanService = () => ({
  getRepositoryConfig: vi.fn(async () => {
    const now = new Date()
    return {
      id: 'repo-1',
      name: 'repo',
      kind: 'remote',
      remote: { url: 'https://example.com/repo.git', provider: 'github' },
      codeExecution: {},
      visibility: 'private',
      createdAt: now,
      updatedAt: now,
    } satisfies RepositoryConfig
  }),
  createProvisionPlan: vi.fn(async () => {
    return {
      mode: 'clone',
      remoteUrl: 'https://example.com/repo.git',
      localTargetPath: '/tmp/repo',
      setupMetadataHash: 'hash123',
    } satisfies WorkspaceProvisionPlan
  }),
})

const makeProvisioner = (overrides?: Partial<WorkspaceProvisioner>): WorkspaceProvisioner => {
  const runningTarget = {
    mode: 'kubernetes',
    podName: 'pod-1',
    podNamespace: 'default',
    repositoryCloneUrl: 'https://example.com/repo.git',
    workerPort: 8001,
    podPhase: 'Running',
  } satisfies ExecutionTarget
  const pendingTarget = {
    mode: 'kubernetes',
    podName: 'pod-1',
    podNamespace: 'default',
    repositoryCloneUrl: 'https://example.com/repo.git',
    workerPort: 8001,
    podPhase: 'Pending',
  } satisfies ExecutionTarget
  const provision: WorkspaceProvisioner['provision'] = async () => ({ ...runningTarget })
  const getInitializingTarget: WorkspaceProvisioner['getInitializingTarget'] = async () => ({
    ...pendingTarget,
  })
  const base: WorkspaceProvisioner = {
    provision: vi.fn(provision),
    getInitializingTarget: vi.fn(getInitializingTarget),
    isHealthy: vi.fn(async () => true),
    validate: vi.fn(async (target: ExecutionTarget) => {
      if (target.mode === 'local') {
        return target.isAvailable !== false
      }
      return target.podPhase === 'Running'
    }),
    refreshExecutionTarget: vi.fn(async (_taskId, target) => target),
    isAgentRunning: vi.fn(async () => false),
    cleanup: vi.fn(async () => undefined),
    clearError: vi.fn(async () => undefined),
    createDriver: vi.fn(() => ({}) as any),
    ...overrides,
  }

  return base
}

describe('WorkspaceRuntimeManager', () => {
  beforeEach(() => {
    vi.restoreAllMocks()
  })

  it('throws on terminal pod errors without reprovisioning', async () => {
    const runtimeStore = new InMemoryRuntimeStore()
    const failingTarget: ExecutionTarget = {
      mode: 'kubernetes',
      podName: 'pod-1',
      podNamespace: 'default',
      repositoryCloneUrl: 'https://example.com/repo.git',
      workerPort: 8001,
      podPhase: 'Failed',
      lastError: 'boom',
    }
    await runtimeStore.save(createWorkspaceSnapshotFromTarget('task-1', 'repo-1', failingTarget))

    const planService = makePlanService()
    const provisioner = makeProvisioner()
    const manager = new WorkspaceRuntimeManager(
      runtimeStore as any,
      planService as any,
      { get: vi.fn() } as any,
      provisioner,
    )

    await expect(manager.ensureWorkspace('task-1')).rejects.toThrow('boom')
    expect(planService.getRepositoryConfig).not.toHaveBeenCalled()
    expect(provisioner.provision).not.toHaveBeenCalled()
  })

  it('provisions when no lock is held and no valid snapshot exists', async () => {
    const runtimeStore = new InMemoryRuntimeStore()
    const planService = makePlanService()
    const provisioner = makeProvisioner()

    const manager = new WorkspaceRuntimeManager(
      runtimeStore as any,
      planService as any,
      { get: vi.fn() } as any,
      provisioner,
    )

    const handle = await manager.ensureWorkspace('task-1', 'repo-1')

    expect(handle.taskId).toBe('task-1')
    expect(provisioner.provision).toHaveBeenCalledTimes(1)
  })

  it('reuses valid existing workspace without reprovisioning', async () => {
    const runtimeStore = new InMemoryRuntimeStore()
    const runningTarget: ExecutionTarget = {
      mode: 'kubernetes',
      podName: 'pod-1',
      podNamespace: 'default',
      repositoryCloneUrl: 'https://example.com/repo.git',
      workerPort: 8001,
      podPhase: 'Running',
    }
    await runtimeStore.save(createWorkspaceSnapshotFromTarget('task-1', 'repo-1', runningTarget))

    const planService = makePlanService()
    const provisioner = makeProvisioner()
    const manager = new WorkspaceRuntimeManager(
      runtimeStore as any,
      planService as any,
      { get: vi.fn() } as any,
      provisioner,
    )

    const handle = await manager.ensureWorkspace('task-1')

    expect(handle.taskId).toBe('task-1')
    expect(provisioner.provision).not.toHaveBeenCalled()
    expect(planService.getRepositoryConfig).not.toHaveBeenCalled()
  })

  it('retries provisioning when setup lock has expired', async () => {
    const runtimeStore = new InMemoryRuntimeStore()
    const pendingTarget: ExecutionTarget = {
      mode: 'kubernetes',
      podName: 'pod-1',
      podNamespace: 'default',
      repositoryCloneUrl: 'https://example.com/repo.git',
      workerPort: 8001,
      podPhase: 'Pending',
    }

    // Create a snapshot with an expired lock (11 minutes old, TTL is 10 minutes)
    const staleSnapshot = createWorkspaceSnapshotFromTarget('task-1', 'repo-1', pendingTarget)
    const expiredLockTime = new Date(Date.now() - 11 * 60 * 1000) // 11 minutes ago
    staleSnapshot.locks = [
      {
        type: 'setup',
        owner: 'other-instance',
        acquiredAt: expiredLockTime,
      },
    ]
    await runtimeStore.save(staleSnapshot)

    const planService = makePlanService()
    const provisioner = makeProvisioner()
    const manager = new WorkspaceRuntimeManager(
      runtimeStore as any,
      planService as any,
      { get: vi.fn() } as any,
      provisioner,
    )

    const handle = await manager.ensureWorkspace('task-1')

    expect(handle.taskId).toBe('task-1')
    // The lock is expired, so the manager should acquire a new lock and provision
    expect(provisioner.provision).toHaveBeenCalledTimes(1)
  })

  it('handles concurrent provisioning attempts by acquiring lock', async () => {
    const runtimeStore = new InMemoryRuntimeStore()
    const planService = makePlanService()

    // First provisioner succeeds
    const provisioner1 = makeProvisioner()

    const manager1 = new WorkspaceRuntimeManager(
      runtimeStore as any,
      planService as any,
      { get: vi.fn() } as any,
      provisioner1,
    )

    // First call should provision successfully
    const handle1 = await manager1.ensureWorkspace('task-1', 'repo-1')
    expect(handle1.taskId).toBe('task-1')
    expect(provisioner1.provision).toHaveBeenCalledTimes(1)

    // Second call with same manager should reuse existing valid workspace
    const handle2 = await manager1.ensureWorkspace('task-1')
    expect(handle2.taskId).toBe('task-1')
    // provision should not be called again since workspace is now valid
    expect(provisioner1.provision).toHaveBeenCalledTimes(1)
  })
})
