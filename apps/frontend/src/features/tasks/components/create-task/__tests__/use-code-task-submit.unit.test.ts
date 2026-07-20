import { renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { CodeTaskSubmitInput } from '../use-code-task-submit'
import { useCodeTaskSubmit } from '../use-code-task-submit'

const createTaskMutate = vi.fn()
const createWorkspaceMutate = vi.fn()

vi.mock('@/features/tasks/hooks/use-tasks', () => ({
  useCreateTask: () => ({ mutate: createTaskMutate, isPending: false }),
}))
vi.mock('@/features/workspaces', () => ({
  useCreateWorkspace: () => ({ mutate: createWorkspaceMutate, isPending: false }),
}))
vi.mock('@tanstack/react-router', () => ({ useNavigate: () => vi.fn() }))
vi.mock('@tanstack/react-query', () => ({
  useQueryClient: () => ({ invalidateQueries: vi.fn() }),
}))

function baseInput(overrides: Partial<CodeTaskSubmitInput> = {}): CodeTaskSubmitInput {
  return {
    agentType: 'claude-code',
    repositoryId: 'repo-api',
    additionalRepos: [],
    repositories: [
      { id: 'repo-api', name: 'api' },
      { id: 'repo-web', name: 'web' },
    ],
    fileIds: [],
    agentMode: 'edit',
    message: 'do the thing',
    model: undefined,
    contexts: [],
    branch: undefined,
    commandName: undefined,
    templateId: undefined,
    onSubmitted: vi.fn(),
    ...overrides,
  }
}

describe('useCodeTaskSubmit', () => {
  beforeEach(() => {
    createTaskMutate.mockReset()
    createWorkspaceMutate.mockReset()
  })

  it('single-repo path creates a task without a workspace', () => {
    const { result } = renderHook(() => useCodeTaskSubmit())

    result.current.submit(baseInput({ additionalRepos: [] }))

    expect(createWorkspaceMutate).not.toHaveBeenCalled()
    expect(createTaskMutate).toHaveBeenCalledTimes(1)
    const payload = createTaskMutate.mock.calls[0]![0]
    expect(payload.repositoryId).toBe('repo-api')
    expect(payload.workspaceId).toBeUndefined()
    expect(payload.message).toBe('do the thing')
  })

  it('multi-repo path creates a workspace then pins the task to it', () => {
    // The workspace mutation resolves with a new workspace id.
    createWorkspaceMutate.mockImplementation((_req, opts) => opts.onSuccess({ id: 'ws-1' }))

    const { result } = renderHook(() => useCodeTaskSubmit())

    result.current.submit(
      baseInput({
        branch: 'main',
        additionalRepos: [{ repositoryId: 'repo-web', branch: 'dev', mountName: 'frontend' }],
      }),
    )

    expect(createWorkspaceMutate).toHaveBeenCalledTimes(1)
    const wsReq = createWorkspaceMutate.mock.calls[0]![0]
    expect(wsReq.name).toBe('api + web')
    expect(wsReq.repos).toHaveLength(2)
    expect(wsReq.repos[0]).toMatchObject({
      repositoryId: 'repo-api',
      isPrimary: true,
      branch: 'main',
    })
    expect(wsReq.repos[1]).toMatchObject({
      repositoryId: 'repo-web',
      position: 1,
      branch: 'dev',
      mountName: 'frontend',
    })

    expect(createTaskMutate).toHaveBeenCalledTimes(1)
    expect(createTaskMutate.mock.calls[0]![0].workspaceId).toBe('ws-1')
  })

  it('drops an extra repo that duplicates the primary (stays single-repo)', () => {
    const { result } = renderHook(() => useCodeTaskSubmit())

    result.current.submit(
      baseInput({
        additionalRepos: [{ repositoryId: 'repo-api', branch: '', mountName: '' }],
      }),
    )

    expect(createWorkspaceMutate).not.toHaveBeenCalled()
    expect(createTaskMutate).toHaveBeenCalledTimes(1)
    expect(createTaskMutate.mock.calls[0]![0].workspaceId).toBeUndefined()
  })
})
