import { CLAUDE_MODELS, DEFAULT_CLAUDE_MODEL } from '@moatless/types'
import { act, renderHook } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import {
  TaskUIProvider,
  useChatMessage,
  useChatModel,
  useChatResumeConversation,
} from '../task-ui-provider'

// Helper to render hooks with TaskUIProvider
const renderHookWithProvider = <T,>(hook: () => T, taskId = 'test-task') => {
  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <TaskUIProvider taskId={taskId}>{children}</TaskUIProvider>
  )
  return renderHook(hook, { wrapper })
}

describe('TaskUIProvider Chat State Management', () => {
  describe('useChatMessage', () => {
    it('should initialize with empty message', () => {
      const { result } = renderHookWithProvider(() => useChatMessage())

      expect(result.current.message).toBe('')
    })

    it('should update message state', () => {
      const { result } = renderHookWithProvider(() => useChatMessage())

      act(() => {
        result.current.setMessage('Hello world')
      })

      expect(result.current.message).toBe('Hello world')
    })

    it('should clear message state', () => {
      const { result } = renderHookWithProvider(() => useChatMessage())

      // Set a message first
      act(() => {
        result.current.setMessage('Test message')
      })

      expect(result.current.message).toBe('Test message')

      // Clear the message
      act(() => {
        result.current.clearMessage()
      })

      expect(result.current.message).toBe('')
    })

    it('should provide stable clearMessage function reference', () => {
      const { result, rerender } = renderHookWithProvider(() => useChatMessage())

      const firstClearMessage = result.current.clearMessage

      rerender()

      const secondClearMessage = result.current.clearMessage

      expect(firstClearMessage).toBe(secondClearMessage)
    })
  })

  describe('useChatModel', () => {
    it('should initialize with default model', () => {
      const { result } = renderHookWithProvider(() => useChatModel())

      expect(result.current.selectedModel).toBe(DEFAULT_CLAUDE_MODEL)
      expect(result.current.selectedModel).toBe(CLAUDE_MODELS.OPUS_4)
    })

    it('should update selected model', () => {
      const { result } = renderHookWithProvider(() => useChatModel())

      act(() => {
        result.current.setSelectedModel(CLAUDE_MODELS.SONNET_4)
      })

      expect(result.current.selectedModel).toBe(CLAUDE_MODELS.SONNET_4)
    })

    it('should allow clearing selected model', () => {
      const { result } = renderHookWithProvider(() => useChatModel())

      act(() => {
        result.current.setSelectedModel(undefined)
      })

      expect(result.current.selectedModel).toBeUndefined()
    })
  })

  describe('useChatResumeConversation', () => {
    it('should initialize with resume conversation enabled', () => {
      const { result } = renderHookWithProvider(() => useChatResumeConversation())

      expect(result.current.resumeConversation).toBe(true)
    })

    it('should update resume conversation setting', () => {
      const { result } = renderHookWithProvider(() => useChatResumeConversation())

      act(() => {
        result.current.setResumeConversation(false)
      })

      expect(result.current.resumeConversation).toBe(false)

      act(() => {
        result.current.setResumeConversation(true)
      })

      expect(result.current.resumeConversation).toBe(true)
    })
  })

  describe('Task isolation', () => {
    it('should isolate state between different task instances', () => {
      // Render hook with first task
      const { result: result1 } = renderHookWithProvider(() => useChatMessage(), 'task-1')

      // Render hook with second task
      const { result: result2 } = renderHookWithProvider(() => useChatMessage(), 'task-2')

      // Set different messages for each task
      act(() => {
        result1.current.setMessage('Message for task 1')
        result2.current.setMessage('Message for task 2')
      })

      // Each task should maintain its own state
      expect(result1.current.message).toBe('Message for task 1')
      expect(result2.current.message).toBe('Message for task 2')

      // Clear message in task 1
      act(() => {
        result1.current.clearMessage()
      })

      // Only task 1 should be cleared
      expect(result1.current.message).toBe('')
      expect(result2.current.message).toBe('Message for task 2')
    })

    it('should isolate model selection between different task instances', () => {
      // Clear localStorage to ensure consistent test behavior
      if (typeof window !== 'undefined') {
        localStorage.removeItem('lastSelectedModel')
      }

      // Render hooks with different tasks
      const { result: result1 } = renderHookWithProvider(() => useChatModel(), 'task-1')
      const { result: result2 } = renderHookWithProvider(() => useChatModel(), 'task-2')

      // Both should start with default model (since localStorage is cleared)
      expect(result1.current.selectedModel).toBe(DEFAULT_CLAUDE_MODEL)
      expect(result2.current.selectedModel).toBe(DEFAULT_CLAUDE_MODEL)

      // Set different models for each task
      act(() => {
        result1.current.setSelectedModel(CLAUDE_MODELS.SONNET_4)
        result2.current.setSelectedModel(CLAUDE_MODELS.OPUS_4)
      })

      // Each task should maintain its own model selection
      expect(result1.current.selectedModel).toBe(CLAUDE_MODELS.SONNET_4)
      expect(result2.current.selectedModel).toBe(CLAUDE_MODELS.OPUS_4)
    })

    it('should isolate resume conversation setting between different task instances', () => {
      // Render hooks with different tasks
      const { result: result1 } = renderHookWithProvider(
        () => useChatResumeConversation(),
        'task-1',
      )
      const { result: result2 } = renderHookWithProvider(
        () => useChatResumeConversation(),
        'task-2',
      )

      // Both should start with default setting (true)
      expect(result1.current.resumeConversation).toBe(true)
      expect(result2.current.resumeConversation).toBe(true)

      // Set different settings for each task
      act(() => {
        result1.current.setResumeConversation(false)
        result2.current.setResumeConversation(true)
      })

      // Each task should maintain its own setting
      expect(result1.current.resumeConversation).toBe(false)
      expect(result2.current.resumeConversation).toBe(true)
    })

    it('should save model selection to localStorage as preference for new tasks', () => {
      // Clear localStorage initially
      if (typeof window !== 'undefined') {
        localStorage.removeItem('lastSelectedModel')
      }

      const { result } = renderHookWithProvider(() => useChatModel(), 'task-1')

      // Change the model
      act(() => {
        result.current.setSelectedModel(CLAUDE_MODELS.SONNET_4)
      })

      // Should save to localStorage
      expect(localStorage.getItem('lastSelectedModel')).toBe(CLAUDE_MODELS.SONNET_4)

      // New task should initialize with the saved preference
      const { result: newTaskResult } = renderHookWithProvider(() => useChatModel(), 'task-2')
      expect(newTaskResult.current.selectedModel).toBe(CLAUDE_MODELS.SONNET_4)
    })
  })
})
