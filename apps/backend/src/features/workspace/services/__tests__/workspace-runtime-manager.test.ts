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

  async touchActivity(taskId: string, timestamp: Date): Promise<void> {
    const snapshot = this.snapshots.get(taskId)
    if (snapshot) {
      this.snapshots.set(taskId, { ...snapshot, lastActivityAt: timestamp, updatedAt: timestamp })
    }
  }
}

const makeEventService = () => ({
  emitEvent: vi.fn(async () => undefined),
})

const makePlanFactory = () => ({
  createPlan: vi.fn(
    async (_taskId: string, _repositoryId: string): Promise<WorkspaceProvisionPlan> => ({
      branch: undefined,
      targetBranch: undefined,
      remoteUrl: 'https://example.com/repo.git',
      provider: 'github',
      pushEnabled: true,
      setupMetadataHash: 'abc123',
      repoConfig: {
        id: 'repo-1',
        name: 'repo',
        kind: 'remote',
        remote: { url: 'https://example.com/repo.git', provider: 'github' },
        codeExecution: {},
        visibility: 'private',
        createdAt: new Date(),
        updatedAt: new Date(),
      } satisfies RepositoryConfig,
      gitCloneUrl: 'https://example.com/repo.git',
      gitBranch: undefined,
      gitProviderToken: 'mock-git-token',
      claudeOAuthToken: 'mock-claude-token',
      anthropicApiKey: undefined,
    }),
  ),
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
  const provision: WorkspaceProvisioner['provision'] = async () => ({ ...runningTarget })
  const base: WorkspaceProvisioner = {
    provision: vi.fn(provision),
    isHealthy: vi.fn(async (_taskId: string, _target: ExecutionTarget) => true),
    getStatus: vi.fn((_taskId: string, target: ExecutionTarget) => {
      // Return status result based on unified status field or podPhase for K8s targets
      let status = target.status
      if (!status && target.mode === 'kubernetes') {
        const k8sTarget = target as { podPhase?: string; podName?: string }
        status = k8sTarget.podPhase === 'Running' ? 'ready' : 'initializing'
      }
      status = status ?? 'initializing'

      return {
        status,
        podName:
          target.mode === 'kubernetes' ? (target as { podName?: string }).podName : undefined,
        containerName:
          target.mode === 'docker'
            ? (target as { containerName?: string }).containerName
            : undefined,
        lastError: target.lastError,
      }
    }),
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

    const provisioner = makeProvisioner()
    const planFactory = makePlanFactory()
    const manager = new WorkspaceRuntimeManager(
      runtimeStore as any,
      { get: vi.fn() } as any,
      provisioner,
      makeEventService() as any,
      planFactory as any,
      {} as any,
    )

    await expect(manager.ensureWorkspace('task-1')).rejects.toThrow('boom')
    expect(planFactory.createPlan).not.toHaveBeenCalled()
    expect(provisioner.provision).not.toHaveBeenCalled()
  })

  it('provisions when no snapshot exists', async () => {
    const runtimeStore = new InMemoryRuntimeStore()
    const provisioner = makeProvisioner()
    const planFactory = makePlanFactory()

    const manager = new WorkspaceRuntimeManager(
      runtimeStore as any,
      { get: vi.fn() } as any,
      provisioner,
      makeEventService() as any,
      planFactory as any,
      {} as any,
    )

    const handle = await manager.ensureWorkspace('task-1', 'repo-1')

    expect(handle.taskId).toBe('task-1')
    expect(planFactory.createPlan).toHaveBeenCalledWith('task-1', 'repo-1')
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

    const provisioner = makeProvisioner()
    const planFactory = makePlanFactory()
    const manager = new WorkspaceRuntimeManager(
      runtimeStore as any,
      { get: vi.fn() } as any,
      provisioner,
      makeEventService() as any,
      planFactory as any,
      {} as any,
    )

    const handle = await manager.ensureWorkspace('task-1')

    expect(handle.taskId).toBe('task-1')
    expect(provisioner.provision).not.toHaveBeenCalled()
    expect(planFactory.createPlan).not.toHaveBeenCalled()
  })

  it('provisions fresh when existing workspace is not ready', async () => {
    const runtimeStore = new InMemoryRuntimeStore()
    const pendingTarget: ExecutionTarget = {
      mode: 'kubernetes',
      podName: 'pod-1',
      podNamespace: 'default',
      repositoryCloneUrl: 'https://example.com/repo.git',
      workerPort: 8001,
      podPhase: 'Pending',
      status: 'not_created',
    }
    await runtimeStore.save(createWorkspaceSnapshotFromTarget('task-1', 'repo-1', pendingTarget))

    const provisioner = makeProvisioner({
      getStatus: vi.fn(() => ({ status: 'not_created' as const, podName: 'pod-1' })),
    })
    const planFactory = makePlanFactory()
    const manager = new WorkspaceRuntimeManager(
      runtimeStore as any,
      { get: vi.fn() } as any,
      provisioner,
      makeEventService() as any,
      planFactory as any,
      {} as any,
    )

    const handle = await manager.ensureWorkspace('task-1', 'repo-1')

    expect(handle.taskId).toBe('task-1')
    expect(planFactory.createPlan).toHaveBeenCalledWith('task-1', 'repo-1')
    expect(provisioner.provision).toHaveBeenCalledTimes(1)
  })

  it('second call reuses existing workspace after provisioning', async () => {
    const runtimeStore = new InMemoryRuntimeStore()
    const provisioner = makeProvisioner()
    const planFactory = makePlanFactory()

    const manager = new WorkspaceRuntimeManager(
      runtimeStore as any,
      { get: vi.fn() } as any,
      provisioner,
      makeEventService() as any,
      planFactory as any,
      {} as any,
    )

    // First call should provision
    const handle1 = await manager.ensureWorkspace('task-1', 'repo-1')
    expect(handle1.taskId).toBe('task-1')
    expect(provisioner.provision).toHaveBeenCalledTimes(1)

    // Second call should reuse existing valid workspace
    const handle2 = await manager.ensureWorkspace('task-1')
    expect(handle2.taskId).toBe('task-1')
    expect(provisioner.provision).toHaveBeenCalledTimes(1)
  })
})
