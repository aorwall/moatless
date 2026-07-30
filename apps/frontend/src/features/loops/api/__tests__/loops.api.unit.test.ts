import {
  AdapterKind,
  type CreateLoopRequest,
  type Loop,
  LoopConfigSource,
  LoopExecutionState,
  LoopKind,
  type LoopRecentTaskItem,
  RoutingMode,
  Scope,
  TaskStatus,
} from '@moatless/api-client'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { loopsApi } from '../loops.api'

const mockListLoops = vi.hoisted(() => vi.fn())
const mockCreateLoop = vi.hoisted(() => vi.fn())
const mockUpdateLoop = vi.hoisted(() => vi.fn())
const mockDeleteLoop = vi.hoisted(() => vi.fn())
const mockGetLoop = vi.hoisted(() => vi.fn())
const mockListLoopRecentTasks = vi.hoisted(() => vi.fn())
const mockActivateLoop = vi.hoisted(() => vi.fn())
const mockPauseLoop = vi.hoisted(() => vi.fn())
const mockResumeLoop = vi.hoisted(() => vi.fn())
const mockOverrideLoop = vi.hoisted(() => vi.fn())
const mockResetGitLoop = vi.hoisted(() => vi.fn())

vi.mock('@moatless/api-client/sandbox/loops/loops', () => ({
  listLoops: mockListLoops,
  createLoop: mockCreateLoop,
  updateLoop: mockUpdateLoop,
  deleteLoop: mockDeleteLoop,
  getLoop: mockGetLoop,
  listLoopRecentTasks: mockListLoopRecentTasks,
  activateLoop: mockActivateLoop,
  pauseLoop: mockPauseLoop,
  resumeLoop: mockResumeLoop,
  overrideLoop: mockOverrideLoop,
  resetGitLoop: mockResetGitLoop,
}))

describe('loopsApi', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('uses generated unified loop handlers', async () => {
    const loopRequest: CreateLoopRequest = {
      kind: LoopKind.adapter_event,
      active: true,
      rule: {
        repositoryId: 'repo-1',
        routingMode: RoutingMode.by_subject,
        prompt: null,
        taskName: null,
        tagIds: [],
      },
      source: {
        adapterKind: AdapterKind.slack,
        connectionId: 'connection-1',
        sourceMatcher: 'C123',
        sourceName: '#triage',
        eventFilter: {},
        adapterConfig: { trigger: 'mention_only' },
      },
    }
    const loopResponse: Loop = {
      id: 'loop-1',
      kind: LoopKind.adapter_event,
      name: null,
      active: true,
      configSource: LoopConfigSource.manual,
      createdBy: 'user-1',
      createdAt: '2026-05-07T10:00:00Z',
      updatedAt: '2026-05-07T11:00:00Z',
      deleted: false,
      executionState: LoopExecutionState.active,
      sourceRepositoryId: null,
      sourceKey: null,
      sourceConfigPath: null,
      syncedFromGit: false,
      scope: Scope.user,
      rule: {
        repositoryId: 'repo-1',
        routingMode: RoutingMode.by_subject,
        ongoingTaskId: null,
        taskName: null,
        prompt: null,
        tagIds: [],
        runAsUserId: null,
        agentType: 'claude-code',
      },
      source: {
        adapterKind: AdapterKind.slack,
        connectionId: 'connection-1',
        sourceMatcher: 'C123',
        sourceName: '#triage',
        eventFilter: {},
        adapterConfig: { trigger: 'mention_only' },
        allowDirectSend: false,
        closeTaskOnClose: true,
        notifyOnSuccess: false,
      },
    }

    mockListLoops.mockResolvedValue([loopResponse])
    mockCreateLoop.mockResolvedValue(loopResponse)
    mockUpdateLoop.mockResolvedValue(loopResponse)
    mockGetLoop.mockResolvedValue(loopResponse)
    mockDeleteLoop.mockResolvedValue(undefined)
    mockActivateLoop.mockResolvedValue(loopResponse)
    mockPauseLoop.mockResolvedValue(loopResponse)
    mockResumeLoop.mockResolvedValue(loopResponse)
    mockOverrideLoop.mockResolvedValue(loopResponse)
    mockResetGitLoop.mockResolvedValue({
      loop: loopResponse,
      resynced: true,
      error: null,
    })

    await expect(loopsApi.listLoops()).resolves.toEqual([loopResponse])
    await expect(loopsApi.createLoop(loopRequest)).resolves.toEqual(loopResponse)
    await expect(loopsApi.updateLoop('loop-1', { name: 'Updated' })).resolves.toEqual(loopResponse)
    await expect(loopsApi.getLoop('loop-1')).resolves.toEqual(loopResponse)
    await expect(loopsApi.deleteLoop('loop-1')).resolves.toBeUndefined()

    expect(mockCreateLoop).toHaveBeenCalledWith(loopRequest)
    await expect(loopsApi.activateLoop('loop-1', { runAsUserId: 'user-1' })).resolves.toEqual(
      loopResponse,
    )
    await expect(loopsApi.pauseLoop('loop-1')).resolves.toEqual(loopResponse)
    await expect(loopsApi.resumeLoop('loop-1')).resolves.toEqual(loopResponse)
    await expect(loopsApi.overrideLoop('loop-1')).resolves.toEqual(loopResponse)
    await expect(loopsApi.resetGitLoop('loop-1')).resolves.toMatchObject({ resynced: true })

    expect(mockUpdateLoop).toHaveBeenCalledWith('loop-1', { name: 'Updated' })
    expect(mockActivateLoop).toHaveBeenCalledWith('loop-1', { runAsUserId: 'user-1' })
  })

  it('lists recent tasks with the requested limit', async () => {
    const recentTask: LoopRecentTaskItem = {
      task: {
        taskId: 'task-1',
        shortId: 'abc123',
        title: 'Triage inbound message',
        status: TaskStatus.open,
        createdAt: '2026-05-07T12:00:00Z',
      },
      binding: null,
    }

    mockListLoopRecentTasks.mockResolvedValue([recentTask])

    await expect(loopsApi.listLoopRecentTasks('loop-1', { limit: 5 })).resolves.toEqual([
      recentTask,
    ])
    expect(mockListLoopRecentTasks).toHaveBeenCalledWith('loop-1', { limit: 5 })
  })
})
