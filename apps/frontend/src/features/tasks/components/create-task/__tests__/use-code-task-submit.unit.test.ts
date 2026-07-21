import { renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { CodeTaskSubmitInput } from '../use-code-task-submit'
import { useCodeTaskSubmit } from '../use-code-task-submit'

const createTaskMutate = vi.fn()

vi.mock('@/features/tasks/hooks/use-tasks', () => ({
  useCreateTask: () => ({ mutate: createTaskMutate, isPending: false }),
}))
vi.mock('@tanstack/react-router', () => ({ useNavigate: () => vi.fn() }))
vi.mock('@tanstack/react-query', () => ({
  useQueryClient: () => ({ invalidateQueries: vi.fn() }),
}))

function baseInput(overrides: Partial<CodeTaskSubmitInput> = {}): CodeTaskSubmitInput {
  return {
    agentType: 'claude-code',
    workspaceId: 'ws-123',
    fileIds: [],
    agentMode: 'edit',
    message: 'do the thing',
    model: undefined,
    contexts: [],
    branch: undefined,
    onSubmitted: vi.fn(),
    ...overrides,
  }
}

describe('useCodeTaskSubmit', () => {
  beforeEach(() => {
    createTaskMutate.mockReset()
  })

  it('creates a task with the selected workspace', () => {
    const { result } = renderHook(() => useCodeTaskSubmit())

    result.current.submit(baseInput())

    expect(createTaskMutate).toHaveBeenCalledTimes(1)
    const payload = createTaskMutate.mock.calls[0]![0]
    expect(payload.workspaceId).toBe('ws-123')
    expect(payload.repositoryId).toBeUndefined()
    expect(payload.message).toBe('do the thing')
  })

  it('includes optional fields when provided', () => {
    const { result } = renderHook(() => useCodeTaskSubmit())

    result.current.submit(
      baseInput({
        branch: 'main',
        model: 'claude-opus-4-8',
      }),
    )

    const payload = createTaskMutate.mock.calls[0]![0]
    expect(payload.branch).toBe('main')
    expect(payload.model).toBe('claude-opus-4-8')
  })

  it('omits undefined optional fields from the payload', () => {
    const { result } = renderHook(() => useCodeTaskSubmit())

    result.current.submit(baseInput({ message: undefined, branch: undefined }))

    const payload = createTaskMutate.mock.calls[0]![0]
    expect(payload).not.toHaveProperty('message')
    expect(payload).not.toHaveProperty('branch')
  })
})
