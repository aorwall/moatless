import { renderHook } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { useFeedbackAction } from '../actions/feedback-action'

describe('useFeedbackAction', () => {
  it('returns an action with the feedback key and a dialog element', () => {
    const { result } = renderHook(() =>
      useFeedbackAction({
        taskId: 'task-1',
        surface: 'message',
        messageUuid: 'msg-1',
      }),
    )
    expect(result.current.action.key).toBe('feedback')
    expect(result.current.action.label).toBe('Give feedback')
    expect(result.current.dialog).toBeDefined()
  })

  it('honors a custom label and threads count into the badge', () => {
    const { result } = renderHook(() =>
      useFeedbackAction({
        taskId: 'task-1',
        surface: 'task',
        label: 'Send feedback',
        count: 3,
      }),
    )
    expect(result.current.action.label).toBe('Send feedback')
    expect(result.current.action.badge).toBeTruthy()
  })
})
