import type { RepositoryConfig } from '@moatless/types'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { SandboxRecord } from '../../db/workspace-sandbox.schema'
import type { WorkspaceProvisionPlan } from '../../types/provision-plan.types'
import type { WorkspaceProvisioner } from '../../types/workspace-provisioner.types'
import type { ExecutionTarget } from '../../types/workspace-runtime.types'
import { WorkspaceRuntimeManager } from '../workspace-runtime-manager.service'

/**
 * In-memory sandbox store for testing.
 * This is the sole persistence layer.
 */
class InMemorySandboxStore {
  private readonly records = new Map<string, SandboxRecord>()

  async register(taskId: string, mode: string, executionTarget: ExecutionTarget): Promise<void> {
    const now = new Date()
    const existing = this.records.get(taskId)
    this.records.set(taskId, {
      taskId,
      mode: mode as 'docker' | 'kubernetes',
      executionTarget,
      lastActivityAt: existing?.lastActivityAt ?? null,
      lastError: executionTarget.lastError ?? null,
      createdAt: existing?.createdAt ?? now,
      updatedAt: now,
    })
  }

  async unregister(taskId: string): Promise<void> {
    this.records.delete(taskId)
  }

  async load(taskId: string): Promise<SandboxRecord | null> {
    return this.records.get(taskId) ?? null
  }

  async exists(taskId: string): Promise<boolean> {
    return this.records.has(taskId)
  }

  async list(): Promise<SandboxRecord[]> {
    return [...this.records.values()]
  }

  async touchActivity(taskId: string, timestamp: Date): Promise<void> {
    const record = this.records.get(taskId)
    if (record) {
      this.records.set(taskId, { ...record, lastActivityAt: timestamp, updatedAt: timestamp })
    }
  }

  async setError(taskId: string, error: string): Promise<void> {
    const record = this.records.get(taskId)
    if (record) {
      this.records.set(taskId, { ...record, lastError: error })
    }
  }

  async clearError(taskId: string): Promise<void> {
    const record = this.records.get(taskId)
    if (record) {
      this.records.set(taskId, { ...record, lastError: null })
    }
  }

  async updateExecutionTarget(taskId: string, target: ExecutionTarget): Promise<void> {
    const record = this.records.get(taskId)
    if (record) {
      this.records.set(taskId, { ...record, executionTarget: target, updatedAt: new Date() })
    }
  }

  // Test helper
  get(taskId: string): SandboxRecord | undefined {
    return this.records.get(taskId)
  }
}

const makeEventService = () => ({
  emitEvent: vi.fn(async () => undefined),
})

const makeTaskRepository = () => ({
  get: vi.fn(async () => ({ repositoryId: 'repo-1' })),
})

const makePlanFactory = () => ({
  createPlan: vi.fn(
    async (_taskId: string, _repositoryId: string): Promise<WorkspaceProvisionPlan> => ({
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
    serviceName: 'moatless-ws-task-1',
    repositoryCloneUrl: 'https://example.com/repo.git',
    workerPort: 8001,
    podPhase: 'Running',
  } satisfies ExecutionTarget
  const initializingTarget = {
    mode: 'kubernetes',
    podNamespace: 'default',
    serviceName: 'moatless-ws-task-1',
    repositoryCloneUrl: 'https://example.com/repo.git',
    workerPort: 8001,
    status: 'initializing',
  } satisfies ExecutionTarget
  const provision: WorkspaceProvisioner['provision'] = async () => ({ ...runningTarget })
  const createInitializingTarget: WorkspaceProvisioner['createInitializingTarget'] = () => ({
    ...initializingTarget,
  })
  const base: WorkspaceProvisioner = {
    createInitializingTarget: vi.fn(createInitializingTarget),
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
    stopSandbox: vi.fn(async () => undefined),
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
    const sandboxStore = new InMemorySandboxStore()
    const failingTarget: ExecutionTarget = {
      mode: 'kubernetes',
      podName: 'pod-1',
      podNamespace: 'default',
      serviceName: 'moatless-ws-task-1',
      repositoryCloneUrl: 'https://example.com/repo.git',
      workerPort: 8001,
      podPhase: 'Failed',
      lastError: 'boom',
    }
    await sandboxStore.register('task-1', 'kubernetes', failingTarget)

    const provisioner = makeProvisioner()
    const planFactory = makePlanFactory()
    const manager = new WorkspaceRuntimeManager(
      sandboxStore as any,
      makeTaskRepository() as any,
      provisioner,
      makeEventService() as any,
      planFactory as any,
    )

    await expect(manager.ensureWorkspace('task-1')).rejects.toThrow('boom')
    expect(planFactory.createPlan).not.toHaveBeenCalled()
    expect(provisioner.provision).not.toHaveBeenCalled()
  })

  it('provisions when no sandbox record exists', async () => {
    const sandboxStore = new InMemorySandboxStore()
    const provisioner = makeProvisioner()
    const planFactory = makePlanFactory()

    const manager = new WorkspaceRuntimeManager(
      sandboxStore as any,
      makeTaskRepository() as any,
      provisioner,
      makeEventService() as any,
      planFactory as any,
    )

    const handle = await manager.ensureWorkspace('task-1', 'repo-1')

    expect(handle.taskId).toBe('task-1')
    expect(planFactory.createPlan).toHaveBeenCalledWith('task-1', 'repo-1')
    expect(provisioner.provision).toHaveBeenCalledTimes(1)
  })

  it('reuses valid existing workspace without reprovisioning', async () => {
    const sandboxStore = new InMemorySandboxStore()
    const runningTarget: ExecutionTarget = {
      mode: 'kubernetes',
      podName: 'pod-1',
      podNamespace: 'default',
      serviceName: 'moatless-ws-task-1',
      repositoryCloneUrl: 'https://example.com/repo.git',
      workerPort: 8001,
      podPhase: 'Running',
    }
    await sandboxStore.register('task-1', 'kubernetes', runningTarget)

    const provisioner = makeProvisioner()
    const planFactory = makePlanFactory()
    const manager = new WorkspaceRuntimeManager(
      sandboxStore as any,
      makeTaskRepository() as any,
      provisioner,
      makeEventService() as any,
      planFactory as any,
    )

    const handle = await manager.ensureWorkspace('task-1')

    expect(handle.taskId).toBe('task-1')
    expect(provisioner.provision).not.toHaveBeenCalled()
    expect(planFactory.createPlan).not.toHaveBeenCalled()
  })

  it('provisions fresh when existing workspace is not ready', async () => {
    const sandboxStore = new InMemorySandboxStore()
    const pendingTarget: ExecutionTarget = {
      mode: 'kubernetes',
      podName: 'pod-1',
      podNamespace: 'default',
      serviceName: 'moatless-ws-task-1',
      repositoryCloneUrl: 'https://example.com/repo.git',
      workerPort: 8001,
      podPhase: 'Pending',
      status: 'not_created',
    }
    await sandboxStore.register('task-1', 'kubernetes', pendingTarget)

    const provisioner = makeProvisioner({
      getStatus: vi.fn(() => ({ status: 'not_created' as const, podName: 'pod-1' })),
    })
    const planFactory = makePlanFactory()
    const manager = new WorkspaceRuntimeManager(
      sandboxStore as any,
      makeTaskRepository() as any,
      provisioner,
      makeEventService() as any,
      planFactory as any,
    )

    const handle = await manager.ensureWorkspace('task-1', 'repo-1')

    expect(handle.taskId).toBe('task-1')
    expect(planFactory.createPlan).toHaveBeenCalledWith('task-1', 'repo-1')
    expect(provisioner.provision).toHaveBeenCalledTimes(1)
  })

  it('second call reuses existing workspace after provisioning', async () => {
    const sandboxStore = new InMemorySandboxStore()
    const provisioner = makeProvisioner()
    const planFactory = makePlanFactory()

    const manager = new WorkspaceRuntimeManager(
      sandboxStore as any,
      makeTaskRepository() as any,
      provisioner,
      makeEventService() as any,
      planFactory as any,
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

  it('saves error state when provisioning fails', async () => {
    const sandboxStore = new InMemorySandboxStore()
    const provisionError = new Error('Mount path not shared with Docker')
    const provisioner = makeProvisioner({
      provision: vi.fn(async () => {
        throw provisionError
      }),
    })
    const planFactory = makePlanFactory()
    const eventService = makeEventService()

    const manager = new WorkspaceRuntimeManager(
      sandboxStore as any,
      makeTaskRepository() as any,
      provisioner,
      eventService as any,
      planFactory as any,
    )

    // Provisioning should fail
    await expect(manager.ensureWorkspace('task-1', 'repo-1')).rejects.toThrow(
      'Mount path not shared with Docker',
    )

    // Verify error state was saved to sandbox record
    const record = sandboxStore.get('task-1')
    expect(record).not.toBeUndefined()
    expect(record!.executionTarget.status).toBe('error')
    expect(record!.executionTarget.lastError).toBe('Mount path not shared with Docker')

    // Verify error event was emitted
    expect(eventService.emitEvent).toHaveBeenCalledWith(
      expect.objectContaining({
        type: 'environment',
        subType: 'error',
        taskId: 'task-1',
        payload: { error: 'Mount path not shared with Docker' },
      }),
    )
  })
})
