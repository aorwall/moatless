import type { RepositoryConfig } from '@moatless/types'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { SandboxControllerProvisioner } from '../../provisioners/sandbox-controller-provisioner'
import type { WorkspaceProvisionPlan } from '../../types/provision-plan.types'
import type { ExecutionTarget } from '../../types/workspace-runtime.types'
import { WorkspaceRuntimeManager } from '../workspace-runtime-manager.service'

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

const makeProvisioner = (
  overrides?: Partial<SandboxControllerProvisioner>,
): SandboxControllerProvisioner => {
  const runningTarget: ExecutionTarget = {
    mode: 'kubernetes',
    podName: 'pod-1',
    podNamespace: 'default',
    serviceName: 'moatless-ws-task-1',
    repositoryCloneUrl: 'https://example.com/repo.git',
    sandboxPort: 8001,
  }
  const initializingTarget: ExecutionTarget = {
    mode: 'kubernetes',
    podNamespace: 'default',
    serviceName: 'moatless-ws-task-1',
    repositoryCloneUrl: 'https://example.com/repo.git',
    sandboxPort: 8001,
    status: 'initializing',
  }
  const provision = async () => ({ ...runningTarget, status: 'ready' as const })
  const createInitializingTarget = () => ({
    ...initializingTarget,
  })
  const base: SandboxControllerProvisioner = {
    createInitializingTarget: vi.fn(createInitializingTarget),
    provision: vi.fn(provision),
    isHealthy: vi.fn(async (_taskId: string, _target: ExecutionTarget) => true),
    getStatus: vi.fn(async () => ({
      status: 'not_created' as const,
    })),
    isAgentRunning: vi.fn(async () => false),
    cleanup: vi.fn(async () => undefined),
    stopSandbox: vi.fn(async () => undefined),
    createDriver: vi.fn(() => ({}) as any),
    getFullWorkspaceStatus: vi.fn(async () => ({
      workspaceState: 'missing',
      environmentStatus: 'not_created',
      agentState: 'none',
      servers: [],
      isRunning: false,
      workspaceReady: false,
      canExecute: false,
    })),
    ...overrides,
  } as any

  return base
}

describe('WorkspaceRuntimeManager', () => {
  beforeEach(() => {
    vi.restoreAllMocks()
  })

  it('throws on terminal error status from sandbox controller', async () => {
    const provisioner = makeProvisioner({
      getStatus: vi.fn(async () => ({
        status: 'error' as const,
        lastError: 'boom',
      })),
    })
    const planFactory = makePlanFactory()
    const manager = new WorkspaceRuntimeManager(
      makeTaskRepository() as any,
      provisioner,
      makeEventService() as any,
      planFactory as any,
      null, // RedisLockService - not used in tests
    )

    await expect(manager.ensureWorkspace('task-1')).rejects.toThrow('boom')
    expect(planFactory.createPlan).not.toHaveBeenCalled()
    expect(provisioner.provision).not.toHaveBeenCalled()
  })

  it('provisions when sandbox controller reports not_created', async () => {
    const provisioner = makeProvisioner({
      getStatus: vi.fn(async () => ({
        status: 'not_created' as const,
      })),
    })
    const planFactory = makePlanFactory()

    const manager = new WorkspaceRuntimeManager(
      makeTaskRepository() as any,
      provisioner,
      makeEventService() as any,
      planFactory as any,
      null,
    )

    const handle = await manager.ensureWorkspace('task-1', 'repo-1')

    expect(handle.taskId).toBe('task-1')
    expect(planFactory.createPlan).toHaveBeenCalledWith('task-1', 'repo-1')
    expect(provisioner.provision).toHaveBeenCalledTimes(1)
  })

  it('reuses valid existing workspace without reprovisioning', async () => {
    const provisioner = makeProvisioner({
      getStatus: vi.fn(async () => ({
        status: 'ready' as const,
        podName: 'pod-1',
      })),
    })
    const planFactory = makePlanFactory()
    const manager = new WorkspaceRuntimeManager(
      makeTaskRepository() as any,
      provisioner,
      makeEventService() as any,
      planFactory as any,
      null,
    )

    const handle = await manager.ensureWorkspace('task-1')

    expect(handle.taskId).toBe('task-1')
    expect(provisioner.provision).not.toHaveBeenCalled()
    expect(planFactory.createPlan).not.toHaveBeenCalled()
  })

  it('provisions fresh when existing workspace is stopped', async () => {
    const provisioner = makeProvisioner({
      getStatus: vi.fn(async () => ({
        status: 'stopped' as const,
      })),
    })
    const planFactory = makePlanFactory()
    const manager = new WorkspaceRuntimeManager(
      makeTaskRepository() as any,
      provisioner,
      makeEventService() as any,
      planFactory as any,
      null,
    )

    const handle = await manager.ensureWorkspace('task-1', 'repo-1')

    expect(handle.taskId).toBe('task-1')
    expect(planFactory.createPlan).toHaveBeenCalledWith('task-1', 'repo-1')
    expect(provisioner.provision).toHaveBeenCalledTimes(1)
  })

  it('second call reuses existing workspace after provisioning', async () => {
    let callCount = 0
    const provisioner = makeProvisioner({
      getStatus: vi.fn(async () => {
        callCount++
        // First call: not_created, subsequent calls: ready (after provisioning)
        if (callCount <= 1) {
          return { status: 'not_created' as const }
        }
        return { status: 'ready' as const, podName: 'pod-1' }
      }),
    })
    const planFactory = makePlanFactory()

    const manager = new WorkspaceRuntimeManager(
      makeTaskRepository() as any,
      provisioner,
      makeEventService() as any,
      planFactory as any,
      null,
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

  it('emits error event when provisioning fails', async () => {
    const provisionError = new Error('Mount path not shared with Docker')
    const provisioner = makeProvisioner({
      getStatus: vi.fn(async () => ({
        status: 'not_created' as const,
      })),
      provision: vi.fn(async () => {
        throw provisionError
      }),
    })
    const planFactory = makePlanFactory()
    const eventService = makeEventService()

    const manager = new WorkspaceRuntimeManager(
      makeTaskRepository() as any,
      provisioner,
      eventService as any,
      planFactory as any,
      null,
    )

    // Provisioning should fail
    await expect(manager.ensureWorkspace('task-1', 'repo-1')).rejects.toThrow(
      'Mount path not shared with Docker',
    )

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
