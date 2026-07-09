import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { useTaskDraftMessage } from '../use-task-draft-message'

const KEY = (taskId: string) => `taskDraftMessage:${taskId}`

describe('useTaskDraftMessage', () => {
  beforeEach(() => {
    localStorage.clear()
  })
  afterEach(() => {
    localStorage.clear()
  })

  it('starts empty when no draft is stored', () => {
    const { result } = renderHook(() => useTaskDraftMessage('task-1'))
    expect(result.current[0]).toBe('')
  })

  it('loads an existing draft for the task on mount', () => {
    localStorage.setItem(KEY('task-1'), 'hello world')
    const { result } = renderHook(() => useTaskDraftMessage('task-1'))
    expect(result.current[0]).toBe('hello world')
  })

  it('persists the draft to localStorage on change', () => {
    const { result } = renderHook(() => useTaskDraftMessage('task-1'))
    act(() => result.current[1]('draft text'))
    expect(result.current[0]).toBe('draft text')
    expect(localStorage.getItem(KEY('task-1'))).toBe('draft text')
  })

  it('scopes drafts by task id', () => {
    const { result } = renderHook(() => useTaskDraftMessage('task-1'))
    act(() => result.current[1]('for task 1'))
    expect(localStorage.getItem(KEY('task-1'))).toBe('for task 1')
    expect(localStorage.getItem(KEY('task-2'))).toBeNull()
  })

  it('loads the correct draft when the task id changes', () => {
    localStorage.setItem(KEY('task-1'), 'draft 1')
    localStorage.setItem(KEY('task-2'), 'draft 2')
    const { result, rerender } = renderHook(({ id }) => useTaskDraftMessage(id), {
      initialProps: { id: 'task-1' },
    })
    expect(result.current[0]).toBe('draft 1')

    rerender({ id: 'task-2' })
    expect(result.current[0]).toBe('draft 2')

    rerender({ id: 'task-1' })
    expect(result.current[0]).toBe('draft 1')
  })

  it('removes the stored draft when cleared to an empty string', () => {
    const { result } = renderHook(() => useTaskDraftMessage('task-1'))
    act(() => result.current[1]('something'))
    expect(localStorage.getItem(KEY('task-1'))).toBe('something')

    act(() => result.current[1](''))
    expect(result.current[0]).toBe('')
    expect(localStorage.getItem(KEY('task-1'))).toBeNull()
  })
})
