import { describe, it, expect, vi } from 'vitest'
import { renderHook, act } from '@testing-library/react'
import { render, screen } from '@testing-library/react'
import { GenericSelector } from '../generic-selector'
import {
  useRepositorySelector,
  useBranchSelector,
  useTemplateSelector,
  useModelSelector
} from '../hooks'
import { CLAUDE_MODELS } from '@moatless/shared/constants'

// Mock the API hooks
vi.mock('@/features/repositories/hooks/use-repositories', () => ({
  useRepositoriesList: vi.fn(() => ({
    data: [
      { id: 'repo1', name: 'Test Repo 1' },
      { id: 'repo2', name: 'Test Repo 2' }
    ],
    isLoading: false
  })),
  useBranches: vi.fn(() => ({
    data: { branches: ['main', 'develop', 'feature/test'] },
    isLoading: false
  }))
}))

vi.mock('@/features/templates', () => ({
  useTemplatesList: vi.fn(() => ({
    data: {
      templates: [
        { id: 'template1', name: 'Template 1' },
        { id: 'template2', name: 'Template 2' }
      ]
    },
    isLoading: false
  }))
}))

describe('Selector Components', () => {
  describe('GenericSelector', () => {
    it('renders with items', () => {
      const onChange = vi.fn()
      const items = [
        { id: '1', label: 'Item 1' },
        { id: '2', label: 'Item 2' }
      ]
      
      render(
        <GenericSelector
          value="1"
          onChange={onChange}
          placeholder="Select item"
          items={items}
        />
      )
      
      expect(screen.getByText('Item 1')).toBeInTheDocument()
    })
  })

  describe('Selector Hooks', () => {
    it('useRepositorySelector returns repository data', () => {
      const { result } = renderHook(() => useRepositorySelector())
      
      expect(result.current.items).toHaveLength(2)
      expect(result.current.items[0]).toEqual({ id: 'repo1', label: 'Test Repo 1' })
      expect(result.current.placeholder).toBe('Select repository')
    })

    it('useBranchSelector returns branch data when repository selected', () => {
      const { result } = renderHook(() => useBranchSelector('repo1'))
      
      expect(result.current.items).toHaveLength(3)
      expect(result.current.items[0]).toEqual({ id: 'main', label: 'main' })
      expect(result.current.disabled).toBe(false)
    })

    it('useBranchSelector is disabled when no repository', () => {
      const { result } = renderHook(() => useBranchSelector())
      
      expect(result.current.disabled).toBe(true)
    })

    it('useTemplateSelector returns template data', () => {
      const { result } = renderHook(() => useTemplateSelector())
      
      expect(result.current.items).toHaveLength(2)
      expect(result.current.items[0]).toEqual({ id: 'template1', label: 'Template 1' })
    })

    it('useModelSelector returns model options', () => {
      const { result } = renderHook(() => useModelSelector())
      
      expect(result.current.items).toHaveLength(2)
      expect(result.current.items.map(i => i.id)).toContain(CLAUDE_MODELS.OPUS_4)
      expect(result.current.items.map(i => i.id)).toContain(CLAUDE_MODELS.SONNET_4)
    })

    it('useModelSelector can change selection', () => {
      const { result } = renderHook(() => useModelSelector())
      
      expect(result.current.value).toBe(CLAUDE_MODELS.OPUS_4)
      
      act(() => {
        result.current.onChange(CLAUDE_MODELS.SONNET_4)
      })
      
      expect(result.current.value).toBe(CLAUDE_MODELS.SONNET_4)
    })
  })
})