import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { SandboxRecord } from '../../db/workspace-sandbox.schema'
import type { ExecutionTarget } from '../../types/workspace-runtime.types'
import { WorkspaceInactivityService } from '../workspace-inactivity.service'

/**
 * In-memory sandbox store for testing.
 * This is the sole store — no runtime store needed.
 */
class InMemorySandboxStore {
  private readonly records = new Map<string, SandboxRecord>()

  async list(): Promise<SandboxRecord[]> {
    return [...this.records.values()]
  }

  async load(taskId: string): Promise<SandboxRecord | null> {
    return this.records.get(taskId) ?? null
  }

  async updateExecutionTarget(taskId: string, target: ExecutionTarget): Promise<void> {
    const record = this.records.get(taskId)
    if (record) {
      this.records.set(taskId, { ...record, executionTarget: target, updatedAt: new Date() })
    }
  }

  /** Helper for tests to set records directly */
  set(record: SandboxRecord): void {
    this.records.set(record.taskId, record)
  }

  get(taskId: string): SandboxRecord | undefined {
    return this.records.get(taskId)
  }
}

function createSandboxRecord(
  overrides: Partial<SandboxRecord> & { executionTarget?: ExecutionTarget } = {},
): SandboxRecord {
  const now = new Date()
  return {
    taskId: 'task-1',
    mode: 'kubernetes',
    executionTarget: {
      mode: 'kubernetes',
      podName: 'pod-task-1',
      podNamespace: 'default',
      repositoryCloneUrl: 'https://github.com/test/repo.git',
      workerPort: 8001,
      status: 'ready',
    },
    lastActivityAt: null,
    lastError: null,
    createdAt: now,
    updatedAt: now,
    ...overrides,
  }
}

describe('WorkspaceInactivityService', () => {
  let service: WorkspaceInactivityService
  let sandboxStore: InMemorySandboxStore
  let runtimeManager: { stopAgentAndInvalidate: ReturnType<typeof vi.fn> }
  let provisioner: {
    clearError: ReturnType<typeof vi.fn>
    cleanup: ReturnType<typeof vi.fn>
    isAgentRunning: ReturnType<typeof vi.fn>
  }
  let originalEnvValue: string | undefined

  beforeEach(() => {
    originalEnvValue = process.env['WORKSPACE_INACTIVITY_TIMEOUT_MINUTES']
    process.env['WORKSPACE_INACTIVITY_TIMEOUT_MINUTES'] = '20'

    sandboxStore = new InMemorySandboxStore()
    runtimeManager = {
      stopAgentAndInvalidate: vi.fn().mockResolvedValue(undefined),
    }
    provisioner = {
      clearError: vi.fn().mockResolvedValue(undefined),
      cleanup: vi.fn().mockResolvedValue(undefined),
      isAgentRunning: vi.fn().mockResolvedValue(false),
    }

    service = new WorkspaceInactivityService(
      runtimeManager as any,
      sandboxStore as any,
      provisioner as any,
    )
  })

  afterEach(() => {
    service.stop()
    if (originalEnvValue === undefined) {
      Reflect.deleteProperty(process.env, 'WORKSPACE_INACTIVITY_TIMEOUT_MINUTES')
    } else {
      process.env['WORKSPACE_INACTIVITY_TIMEOUT_MINUTES'] = originalEnvValue
    }
  })

  it('suspends inactive workspace', async () => {
    const thirtyMinAgo = new Date(Date.now() - 30 * 60 * 1000)
    sandboxStore.set(
      createSandboxRecord({
        lastActivityAt: thirtyMinAgo,
        updatedAt: thirtyMinAgo,
      }),
    )

    await service.checkInactiveWorkspaces()

    const record = sandboxStore.get('task-1')
    expect(record?.executionTarget.status).toBe('stopped')
    expect(provisioner.clearError).toHaveBeenCalledWith(
      'task-1',
      expect.objectContaining({ mode: 'kubernetes' }),
    )
  })

  it('does not suspend active workspace', async () => {
    const fiveMinAgo = new Date(Date.now() - 5 * 60 * 1000)
    sandboxStore.set(
      createSandboxRecord({
        lastActivityAt: fiveMinAgo,
        updatedAt: fiveMinAgo,
      }),
    )

    await service.checkInactiveWorkspaces()

    const record = sandboxStore.get('task-1')
    expect(record?.executionTarget.status).toBe('ready')
    expect(provisioner.clearError).not.toHaveBeenCalled()
  })

  it('does not suspend workspace with running agent', async () => {
    const thirtyMinAgo = new Date(Date.now() - 30 * 60 * 1000)
    sandboxStore.set(
      createSandboxRecord({
        lastActivityAt: thirtyMinAgo,
        updatedAt: thirtyMinAgo,
      }),
    )
    // Agent is running — live query returns true
    provisioner.isAgentRunning.mockResolvedValue(true)

    await service.checkInactiveWorkspaces()

    const record = sandboxStore.get('task-1')
    expect(record?.executionTarget.status).toBe('ready')
    expect(provisioner.clearError).not.toHaveBeenCalled()
  })

  it('skips non-ready workspace', async () => {
    const thirtyMinAgo = new Date(Date.now() - 30 * 60 * 1000)
    sandboxStore.set(
      createSandboxRecord({
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
    process.env['WORKSPACE_INACTIVITY_TIMEOUT_MINUTES'] = '0'

    const thirtyMinAgo = new Date(Date.now() - 30 * 60 * 1000)
    sandboxStore.set(
      createSandboxRecord({
        lastActivityAt: thirtyMinAgo,
        updatedAt: thirtyMinAgo,
      }),
    )

    await service.checkInactiveWorkspaces()

    const record = sandboxStore.get('task-1')
    expect(record?.executionTarget.status).toBe('ready')
    expect(provisioner.clearError).not.toHaveBeenCalled()
  })

  it('errors in one workspace do not prevent checking others', async () => {
    const thirtyMinAgo = new Date(Date.now() - 30 * 60 * 1000)

    // First workspace will throw during clearError
    sandboxStore.set(
      createSandboxRecord({
        taskId: 'task-fail',
        lastActivityAt: thirtyMinAgo,
        updatedAt: thirtyMinAgo,
      }),
    )
    provisioner.clearError.mockRejectedValueOnce(new Error('pod not found'))

    // Second workspace should still be processed
    sandboxStore.set(
      createSandboxRecord({
        taskId: 'task-ok',
        lastActivityAt: thirtyMinAgo,
        updatedAt: thirtyMinAgo,
      }),
    )

    await service.checkInactiveWorkspaces()

    // task-fail still gets status update despite clearError failure
    const failRecord = sandboxStore.get('task-fail')
    expect(failRecord?.executionTarget.status).toBe('stopped')

    // task-ok is also suspended
    const okRecord = sandboxStore.get('task-ok')
    expect(okRecord?.executionTarget.status).toBe('stopped')
  })

  it('updates record status to stopped after suspend', async () => {
    const thirtyMinAgo = new Date(Date.now() - 30 * 60 * 1000)
    sandboxStore.set(
      createSandboxRecord({
        lastActivityAt: thirtyMinAgo,
        updatedAt: thirtyMinAgo,
      }),
    )

    await service.checkInactiveWorkspaces()

    const record = sandboxStore.get('task-1')
    expect(record?.executionTarget.status).toBe('stopped')
    // Record is preserved (not deleted)
    expect(record?.taskId).toBe('task-1')
  })

  it('uses clearError for Kubernetes (pod-only deletion)', async () => {
    const thirtyMinAgo = new Date(Date.now() - 30 * 60 * 1000)
    sandboxStore.set(
      createSandboxRecord({
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
    sandboxStore.set(
      createSandboxRecord({
        taskId: 'task-1',
        mode: 'docker',
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
    sandboxStore.set(
      createSandboxRecord({
        lastActivityAt: null,
        updatedAt: thirtyMinAgo,
      }),
    )

    await service.checkInactiveWorkspaces()

    const record = sandboxStore.get('task-1')
    expect(record?.executionTarget.status).toBe('stopped')
  })

  it('calls stopAgentAndInvalidate before stopping resources', async () => {
    const thirtyMinAgo = new Date(Date.now() - 30 * 60 * 1000)
    sandboxStore.set(
      createSandboxRecord({
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
    process.env['WORKSPACE_INACTIVITY_TIMEOUT_MINUTES'] = '0'

    const setIntervalSpy = vi.spyOn(globalThis, 'setInterval')
    service.start()

    expect(setIntervalSpy).not.toHaveBeenCalled()
    setIntervalSpy.mockRestore()
  })
})
